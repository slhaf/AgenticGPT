use crate::browser_kernel::NodeReplKernel;
use crate::browser_runtime::NodeReplLaunchSpec;
use anyhow::{anyhow, Result};
use rmcp::model::CallToolResult;
use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify};
use tokio::time::MissedTickBehavior;
use uuid::Uuid;

const REAPER_INTERVAL: Duration = Duration::from_secs(1);

type KernelFuture = Pin<Box<dyn Future<Output = Result<ManagedKernel>> + Send + 'static>>;
type KernelFactory = Arc<dyn Fn(String, String) -> KernelFuture + Send + Sync>;

enum ManagedKernel {
    Node(NodeReplKernel),
    #[cfg(test)]
    Fake(FakeKernel),
}

impl ManagedKernel {
    async fn js(&mut self, code: &str, timeout_ms: u64) -> Result<CallToolResult> {
        match self {
            Self::Node(kernel) => kernel.js(code, timeout_ms).await,
            #[cfg(test)]
            Self::Fake(kernel) => kernel.js(code, timeout_ms).await,
        }
    }

    fn is_closed(&self) -> bool {
        match self {
            Self::Node(kernel) => kernel.is_closed(),
            #[cfg(test)]
            Self::Fake(kernel) => kernel.is_closed(),
        }
    }

    async fn shutdown(self) -> Result<()> {
        match self {
            Self::Node(kernel) => kernel.shutdown().await,
            #[cfg(test)]
            Self::Fake(kernel) => kernel.shutdown().await,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BrowserLeaseState {
    Initializing,
    Ready,
    Closing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BrowserLeaseSnapshot {
    pub(crate) name: String,
    pub(crate) state: BrowserLeaseState,
    pub(crate) idle_timeout: Duration,
    pub(crate) remaining_idle: Option<Duration>,
}

struct LeaseEntry {
    lifecycle: Mutex<LeaseLifecycle>,
    changed: Notify,
}

struct LeaseLifecycle {
    state: BrowserLeaseState,
    idle_timeout: Duration,
    last_activity: Instant,
    kernel: Option<ManagedKernel>,
}

impl LeaseEntry {
    fn initializing(idle_timeout: Duration) -> Self {
        Self {
            lifecycle: Mutex::new(LeaseLifecycle {
                state: BrowserLeaseState::Initializing,
                idle_timeout,
                last_activity: Instant::now(),
                kernel: None,
            }),
            changed: Notify::new(),
        }
    }
}

pub(crate) struct BrowserRuntimeManager {
    entries: Mutex<BTreeMap<String, Arc<LeaseEntry>>>,
    factory: KernelFactory,
}

impl BrowserRuntimeManager {
    pub(crate) fn new(spec: NodeReplLaunchSpec, browser_client_path: PathBuf) -> Arc<Self> {
        let factory: KernelFactory = Arc::new(move |session_id, turn_id| {
            let spec = spec.clone();
            let browser_client_path = browser_client_path.clone();
            Box::pin(async move {
                let mut kernel = NodeReplKernel::spawn(&spec, session_id, turn_id).await?;
                if let Err(error) = kernel.bootstrap_browser(&browser_client_path).await {
                    let _ = kernel.shutdown().await;
                    return Err(error);
                }
                Ok(ManagedKernel::Node(kernel))
            })
        });
        Self::with_factory(factory, REAPER_INTERVAL)
    }

    fn with_factory(factory: KernelFactory, reaper_interval: Duration) -> Arc<Self> {
        let manager = Arc::new(Self {
            entries: Mutex::new(BTreeMap::new()),
            factory,
        });
        let weak = Arc::downgrade(&manager);
        tokio::spawn(reaper_loop(weak, reaper_interval));
        manager
    }

    pub(crate) async fn acquire(&self, name: &str, idle_timeout: Duration) -> Result<()> {
        validate_lease_name(name)?;
        if idle_timeout.is_zero() {
            return Err(anyhow!("browser_runtime_idle_timeout_invalid"));
        }

        loop {
            let (entry, initialize) = {
                let mut entries = self.entries.lock().await;
                if let Some(entry) = entries.get(name).cloned() {
                    (entry, false)
                } else {
                    let entry = Arc::new(LeaseEntry::initializing(idle_timeout));
                    entries.insert(name.to_owned(), entry.clone());
                    (entry, true)
                }
            };

            if initialize {
                return self.initialize(name, entry).await;
            }

            let notified = entry.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let mut lifecycle = entry.lifecycle.lock().await;
            match lifecycle.state {
                BrowserLeaseState::Initializing | BrowserLeaseState::Closing => {
                    drop(lifecycle);
                    if !self.is_current(name, &entry).await {
                        continue;
                    }
                    notified.await;
                }
                BrowserLeaseState::Ready => {
                    let closed = lifecycle
                        .kernel
                        .as_ref()
                        .map(ManagedKernel::is_closed)
                        .unwrap_or(true);
                    if closed {
                        lifecycle.state = BrowserLeaseState::Closing;
                        let kernel = lifecycle.kernel.take();
                        drop(lifecycle);
                        if let Some(kernel) = kernel {
                            let _ = kernel.shutdown().await;
                        }
                        self.remove_exact(name, &entry).await;
                    } else {
                        lifecycle.idle_timeout = idle_timeout;
                        lifecycle.last_activity = Instant::now();
                        return Ok(());
                    }
                }
            }
        }
    }

    async fn initialize(&self, name: &str, entry: Arc<LeaseEntry>) -> Result<()> {
        let result = (self.factory)(Uuid::new_v4().to_string(), Uuid::new_v4().to_string()).await;
        match result {
            Ok(kernel) => {
                let mut lifecycle = entry.lifecycle.lock().await;
                lifecycle.state = BrowserLeaseState::Ready;
                lifecycle.kernel = Some(kernel);
                lifecycle.last_activity = Instant::now();
                drop(lifecycle);
                entry.changed.notify_waiters();
                Ok(())
            }
            Err(error) => {
                self.remove_exact(name, &entry).await;
                Err(error)
            }
        }
    }

    pub(crate) async fn repl(
        &self,
        name: &str,
        code: &str,
        timeout_ms: u64,
    ) -> Result<CallToolResult> {
        validate_lease_name(name)?;
        let entry = self
            .entry(name)
            .await
            .ok_or_else(|| anyhow!("browser_runtime_lease_not_found"))?;

        loop {
            let notified = entry.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let mut lifecycle = entry.lifecycle.lock().await;
            match lifecycle.state {
                BrowserLeaseState::Initializing | BrowserLeaseState::Closing => {
                    drop(lifecycle);
                    if !self.is_current(name, &entry).await {
                        return Err(anyhow!("browser_runtime_lease_not_found"));
                    }
                    notified.await;
                    if !self.is_current(name, &entry).await {
                        return Err(anyhow!("browser_runtime_lease_not_found"));
                    }
                }
                BrowserLeaseState::Ready => {
                    lifecycle.last_activity = Instant::now();
                    let result = match lifecycle.kernel.as_mut() {
                        Some(kernel) => kernel.js(code, timeout_ms).await,
                        None => Err(anyhow!("browser_runtime_lease_not_found")),
                    };
                    lifecycle.last_activity = Instant::now();
                    return result;
                }
            }
        }
    }

    pub(crate) async fn release(&self, name: &str) -> Result<bool> {
        validate_lease_name(name)?;
        let Some(entry) = self.entry(name).await else {
            return Ok(false);
        };

        loop {
            let notified = entry.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let mut lifecycle = entry.lifecycle.lock().await;
            match lifecycle.state {
                BrowserLeaseState::Initializing | BrowserLeaseState::Closing => {
                    drop(lifecycle);
                    if !self.is_current(name, &entry).await {
                        return Ok(false);
                    }
                    notified.await;
                    if !self.is_current(name, &entry).await {
                        return Ok(false);
                    }
                }
                BrowserLeaseState::Ready => {
                    lifecycle.state = BrowserLeaseState::Closing;
                    let kernel = lifecycle.kernel.take();
                    drop(lifecycle);
                    let result = match kernel {
                        Some(kernel) => kernel.shutdown().await,
                        None => Ok(()),
                    };
                    self.remove_exact(name, &entry).await;
                    return result.map(|()| true);
                }
            }
        }
    }

    pub(crate) async fn list(&self) -> Vec<BrowserLeaseSnapshot> {
        let entries = {
            let entries = self.entries.lock().await;
            entries
                .iter()
                .map(|(name, entry)| (name.clone(), entry.clone()))
                .collect::<Vec<_>>()
        };
        let now = Instant::now();
        let mut snapshots = Vec::with_capacity(entries.len());
        for (name, entry) in entries {
            let lifecycle = entry.lifecycle.lock().await;
            let remaining_idle = match lifecycle.state {
                BrowserLeaseState::Ready => {
                    Some(deadline(&lifecycle).saturating_duration_since(now))
                }
                BrowserLeaseState::Initializing | BrowserLeaseState::Closing => None,
            };
            snapshots.push(BrowserLeaseSnapshot {
                name,
                state: lifecycle.state,
                idle_timeout: lifecycle.idle_timeout,
                remaining_idle,
            });
        }
        snapshots
    }

    async fn entry(&self, name: &str) -> Option<Arc<LeaseEntry>> {
        self.entries.lock().await.get(name).cloned()
    }

    async fn is_current(&self, name: &str, entry: &Arc<LeaseEntry>) -> bool {
        self.entries
            .lock()
            .await
            .get(name)
            .is_some_and(|current| Arc::ptr_eq(current, entry))
    }

    async fn remove_exact(&self, name: &str, entry: &Arc<LeaseEntry>) -> bool {
        let removed = {
            let mut entries = self.entries.lock().await;
            match entries.get(name) {
                Some(current) if Arc::ptr_eq(current, entry) => entries.remove(name).is_some(),
                _ => false,
            }
        };
        if removed {
            entry.changed.notify_waiters();
        }
        removed
    }

    async fn reap_expired(&self) {
        let entries = {
            let entries = self.entries.lock().await;
            entries
                .iter()
                .map(|(name, entry)| (name.clone(), entry.clone()))
                .collect::<Vec<_>>()
        };

        for (name, entry) in entries {
            let mut lifecycle = entry.lifecycle.lock().await;
            if lifecycle.state != BrowserLeaseState::Ready || Instant::now() < deadline(&lifecycle)
            {
                continue;
            }
            lifecycle.state = BrowserLeaseState::Closing;
            let kernel = lifecycle.kernel.take();
            drop(lifecycle);
            if let Some(kernel) = kernel {
                let _ = kernel.shutdown().await;
            }
            self.remove_exact(&name, &entry).await;
        }
    }
}

async fn reaper_loop(manager: Weak<BrowserRuntimeManager>, reaper_interval: Duration) {
    let mut ticker = tokio::time::interval(reaper_interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        let Some(manager) = manager.upgrade() else {
            return;
        };
        manager.reap_expired().await;
    }
}

fn deadline(lifecycle: &LeaseLifecycle) -> Instant {
    lifecycle
        .last_activity
        .checked_add(lifecycle.idle_timeout)
        .unwrap_or(lifecycle.last_activity)
}

fn validate_lease_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(anyhow!("browser_runtime_lease_name_invalid"));
    }
    Ok(())
}

#[cfg(test)]
struct FakeController {
    block_js: std::sync::atomic::AtomicBool,
    block_shutdown: std::sync::atomic::AtomicBool,
    call_count: std::sync::atomic::AtomicUsize,
    active_calls: std::sync::atomic::AtomicUsize,
    max_active_calls: std::sync::atomic::AtomicUsize,
    entered_count: std::sync::atomic::AtomicUsize,
    entered: Notify,
    release_js: Notify,
    shutdown_count: std::sync::atomic::AtomicUsize,
    shutdowns: Notify,
    release_shutdown: Notify,
}

#[cfg(test)]
impl FakeController {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            block_js: std::sync::atomic::AtomicBool::new(false),
            block_shutdown: std::sync::atomic::AtomicBool::new(false),
            call_count: std::sync::atomic::AtomicUsize::new(0),
            active_calls: std::sync::atomic::AtomicUsize::new(0),
            max_active_calls: std::sync::atomic::AtomicUsize::new(0),
            entered_count: std::sync::atomic::AtomicUsize::new(0),
            entered: Notify::new(),
            release_js: Notify::new(),
            shutdown_count: std::sync::atomic::AtomicUsize::new(0),
            shutdowns: Notify::new(),
            release_shutdown: Notify::new(),
        })
    }
}

#[cfg(test)]
struct FakeKernel {
    controller: Arc<FakeController>,
    closed: Arc<std::sync::atomic::AtomicBool>,
    shutdown_error: bool,
    js_error: bool,
    result: CallToolResult,
}

#[cfg(test)]
impl FakeKernel {
    fn new(controller: Arc<FakeController>) -> Self {
        Self {
            controller,
            closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            shutdown_error: false,
            js_error: false,
            result: CallToolResult::default(),
        }
    }

    async fn js(&mut self, _code: &str, _timeout_ms: u64) -> Result<CallToolResult> {
        use std::sync::atomic::Ordering;

        let controller = &self.controller;
        controller.call_count.fetch_add(1, Ordering::SeqCst);
        let active = controller.active_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let mut observed = controller.max_active_calls.load(Ordering::SeqCst);
        while active > observed {
            match controller.max_active_calls.compare_exchange(
                observed,
                active,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(current) => observed = current,
            }
        }
        controller.entered_count.fetch_add(1, Ordering::SeqCst);
        controller.entered.notify_waiters();

        if controller.block_js.load(Ordering::SeqCst) {
            controller.release_js.notified().await;
        }
        controller.active_calls.fetch_sub(1, Ordering::SeqCst);
        if self.js_error {
            Err(anyhow!("fake js failure"))
        } else {
            Ok(self.result.clone())
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(std::sync::atomic::Ordering::SeqCst)
    }

    async fn shutdown(self) -> Result<()> {
        use std::sync::atomic::Ordering;

        self.closed.store(true, Ordering::SeqCst);
        self.controller
            .shutdown_count
            .fetch_add(1, Ordering::SeqCst);
        self.controller.shutdowns.notify_waiters();
        if self.controller.block_shutdown.load(Ordering::SeqCst) {
            self.controller.release_shutdown.notified().await;
        }
        if self.shutdown_error {
            Err(anyhow!("fake shutdown failed"))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::{Barrier, Notify};
    use tokio::time::{sleep, timeout};

    async fn wait_for_count(counter: &AtomicUsize, notify: &Notify, expected: usize) {
        loop {
            let notified = notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if counter.load(Ordering::SeqCst) >= expected {
                return;
            }
            notified.await;
        }
    }

    fn fake_factory<F>(make: F) -> (KernelFactory, Arc<AtomicUsize>)
    where
        F: Fn(usize) -> FakeKernel + Send + Sync + 'static,
    {
        let calls = Arc::new(AtomicUsize::new(0));
        let factory_calls = calls.clone();
        let factory: KernelFactory = Arc::new(move |_, _| {
            let index = factory_calls.fetch_add(1, Ordering::SeqCst);
            let kernel = make(index);
            Box::pin(async move { Ok(ManagedKernel::Fake(kernel)) })
        });
        (factory, calls)
    }

    fn manager(factory: KernelFactory, reaper_interval: Duration) -> Arc<BrowserRuntimeManager> {
        BrowserRuntimeManager::with_factory(factory, reaper_interval)
    }

    #[tokio::test]
    async fn concurrent_same_name_acquire_shares_one_initialization() {
        let controller = FakeController::new();
        let factory_calls = Arc::new(AtomicUsize::new(0));
        let factory_entered = Arc::new(Notify::new());
        let factory_release = Arc::new(Notify::new());
        let factory: KernelFactory = {
            let factory_calls = factory_calls.clone();
            let factory_entered = factory_entered.clone();
            let factory_release = factory_release.clone();
            let controller = controller.clone();
            Arc::new(move |_, _| {
                factory_calls.fetch_add(1, Ordering::SeqCst);
                factory_entered.notify_waiters();
                let factory_entered = factory_entered.clone();
                let factory_release = factory_release.clone();
                let controller = controller.clone();
                Box::pin(async move {
                    factory_entered.notify_waiters();
                    factory_release.notified().await;
                    Ok(ManagedKernel::Fake(FakeKernel::new(controller)))
                })
            })
        };
        let manager = manager(factory, Duration::from_millis(10));

        let first_manager = manager.clone();
        let first =
            tokio::spawn(
                async move { first_manager.acquire("same", Duration::from_secs(1)).await },
            );
        let entered_wait = timeout(
            Duration::from_secs(1),
            wait_for_count(&factory_calls, &factory_entered, 1),
        );
        entered_wait.await.unwrap();

        let second_manager = manager.clone();
        let second =
            tokio::spawn(
                async move { second_manager.acquire("same", Duration::from_secs(2)).await },
            );
        tokio::task::yield_now().await;
        assert_eq!(factory_calls.load(Ordering::SeqCst), 1);

        factory_release.notify_one();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();

        let snapshots = manager.list().await;
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].name, "same");
        assert_eq!(snapshots[0].idle_timeout, Duration::from_secs(2));
        manager.release("same").await.unwrap();
    }

    #[tokio::test]
    async fn different_names_initialize_concurrently() {
        let controller = FakeController::new();
        let barrier = Arc::new(Barrier::new(2));
        let (factory, calls) = fake_factory({
            let controller = controller.clone();
            move |_| FakeKernel {
                controller: controller.clone(),
                closed: Arc::new(AtomicBool::new(false)),
                shutdown_error: false,
                js_error: false,
                result: CallToolResult::default(),
            }
        });
        let factory: KernelFactory = {
            let original = factory;
            let barrier = barrier.clone();
            Arc::new(move |_, _| {
                let barrier = barrier.clone();
                let future = original("session".to_string(), "turn".to_string());
                Box::pin(async move {
                    barrier.wait().await;
                    future.await
                })
            })
        };
        let manager = manager(factory, Duration::from_millis(10));
        let left_manager = manager.clone();
        let right_manager = manager.clone();
        let joined = timeout(Duration::from_secs(2), async move {
            tokio::join!(
                left_manager.acquire("left", Duration::from_secs(1)),
                right_manager.acquire("right", Duration::from_secs(1)),
            )
        })
        .await
        .unwrap();
        joined.0.unwrap();
        joined.1.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        manager.release("left").await.unwrap();
        manager.release("right").await.unwrap();
    }

    #[tokio::test]
    async fn repl_serializes_per_lease_and_overlaps_across_leases() {
        let controller = FakeController::new();
        controller.block_js.store(true, Ordering::SeqCst);
        let (factory, _) = fake_factory({
            let controller = controller.clone();
            move |_| FakeKernel::new(controller.clone())
        });
        let manager = manager(factory, Duration::from_secs(1));
        manager
            .acquire("one", Duration::from_secs(1))
            .await
            .unwrap();
        manager
            .acquire("two", Duration::from_secs(1))
            .await
            .unwrap();

        let first_manager = manager.clone();
        let first = tokio::spawn(async move { first_manager.repl("one", "first", 1).await });
        timeout(
            Duration::from_secs(1),
            wait_for_count(&controller.entered_count, &controller.entered, 1),
        )
        .await
        .unwrap();

        let second_manager = manager.clone();
        let second = tokio::spawn(async move { second_manager.repl("one", "second", 1).await });
        tokio::task::yield_now().await;
        assert_eq!(controller.call_count.load(Ordering::SeqCst), 1);

        controller.release_js.notify_one();
        first.await.unwrap().unwrap();
        timeout(
            Duration::from_secs(1),
            wait_for_count(&controller.entered_count, &controller.entered, 2),
        )
        .await
        .unwrap();
        controller.release_js.notify_one();
        second.await.unwrap().unwrap();

        controller.entered_count.store(0, Ordering::SeqCst);
        controller.call_count.store(0, Ordering::SeqCst);
        controller.max_active_calls.store(0, Ordering::SeqCst);
        let left_manager = manager.clone();
        let left = tokio::spawn(async move { left_manager.repl("one", "left", 1).await });
        let right_manager = manager.clone();
        let right = tokio::spawn(async move { right_manager.repl("two", "right", 1).await });
        timeout(
            Duration::from_secs(1),
            wait_for_count(&controller.entered_count, &controller.entered, 2),
        )
        .await
        .unwrap();
        assert!(controller.max_active_calls.load(Ordering::SeqCst) >= 2);
        controller.release_js.notify_one();
        controller.release_js.notify_one();
        left.await.unwrap().unwrap();
        right.await.unwrap().unwrap();
        controller.block_js.store(false, Ordering::SeqCst);
        manager.release("one").await.unwrap();
        manager.release("two").await.unwrap();
    }

    #[tokio::test]
    async fn failed_initialization_removes_entry_and_allows_retry() {
        let controller = FakeController::new();
        let attempts = Arc::new(AtomicUsize::new(0));
        let factory: KernelFactory = {
            let attempts = attempts.clone();
            let controller = controller.clone();
            Arc::new(move |_, _| {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                let controller = controller.clone();
                Box::pin(async move {
                    if attempt == 0 {
                        Err(anyhow!("fake initialization failed"))
                    } else {
                        Ok(ManagedKernel::Fake(FakeKernel::new(controller)))
                    }
                })
            })
        };
        let manager = manager(factory, Duration::from_secs(1));

        let error = manager
            .acquire("retry", Duration::from_secs(1))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "fake initialization failed");
        assert!(manager.list().await.is_empty());
        manager
            .acquire("retry", Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        manager.release("retry").await.unwrap();
    }

    #[tokio::test]
    async fn release_removes_after_shutdown_error_and_reacquire_is_fresh() {
        let controller = FakeController::new();
        let (factory, calls) = fake_factory({
            let controller = controller.clone();
            move |index| {
                let mut kernel = FakeKernel::new(controller.clone());
                kernel.shutdown_error = index == 0;
                kernel
            }
        });
        let manager = manager(factory, Duration::from_secs(1));
        manager
            .acquire("release", Duration::from_secs(1))
            .await
            .unwrap();
        let error = manager.release("release").await.unwrap_err();
        assert_eq!(error.to_string(), "fake shutdown failed");
        assert!(manager.list().await.is_empty());

        manager
            .acquire("release", Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(manager.release("release").await.unwrap());
        assert!(!manager.release("release").await.unwrap());
    }

    #[tokio::test]
    async fn acquire_waits_for_closing_release_then_creates_fresh_kernel() {
        let controller = FakeController::new();
        controller.block_shutdown.store(true, Ordering::SeqCst);
        let (factory, calls) = fake_factory({
            let controller = controller.clone();
            move |_| FakeKernel::new(controller.clone())
        });
        let manager = manager(factory, Duration::from_secs(1));
        manager
            .acquire("closing", Duration::from_secs(1))
            .await
            .unwrap();

        let release_manager = manager.clone();
        let release = tokio::spawn(async move { release_manager.release("closing").await });
        timeout(
            Duration::from_secs(1),
            wait_for_count(&controller.shutdown_count, &controller.shutdowns, 1),
        )
        .await
        .unwrap();

        let acquire_manager = manager.clone();
        let acquire = tokio::spawn(async move {
            acquire_manager
                .acquire("closing", Duration::from_secs(2))
                .await
        });
        tokio::task::yield_now().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        controller.release_shutdown.notify_one();
        assert!(release.await.unwrap().unwrap());
        acquire.await.unwrap().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        controller.block_shutdown.store(false, Ordering::SeqCst);
        assert!(manager.release("closing").await.unwrap());
    }

    #[tokio::test]
    async fn idle_reaper_removes_inactive_lease() {
        let controller = FakeController::new();
        let (factory, _) = fake_factory({
            let controller = controller.clone();
            move |_| FakeKernel::new(controller.clone())
        });
        let manager = manager(factory, Duration::from_millis(5));
        manager
            .acquire("idle", Duration::from_millis(25))
            .await
            .unwrap();
        timeout(
            Duration::from_secs(2),
            wait_for_count(&controller.shutdown_count, &controller.shutdowns, 1),
        )
        .await
        .unwrap();
        assert!(manager.list().await.is_empty());
    }

    #[tokio::test]
    async fn repl_activity_wins_race_with_idle_reaper() {
        let controller = FakeController::new();
        controller.block_js.store(true, Ordering::SeqCst);
        let (factory, _) = fake_factory({
            let controller = controller.clone();
            move |_| FakeKernel::new(controller.clone())
        });
        let manager = manager(factory, Duration::from_millis(5));
        manager
            .acquire("active", Duration::from_millis(25))
            .await
            .unwrap();
        let repl_manager = manager.clone();
        let repl = tokio::spawn(async move { repl_manager.repl("active", "code", 1).await });
        timeout(
            Duration::from_secs(1),
            wait_for_count(&controller.entered_count, &controller.entered, 1),
        )
        .await
        .unwrap();

        sleep(Duration::from_millis(100)).await;
        assert_eq!(controller.shutdown_count.load(Ordering::SeqCst), 0);
        controller.release_js.notify_one();
        repl.await.unwrap().unwrap();
        controller.block_js.store(false, Ordering::SeqCst);
        let snapshots = manager.list().await;
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].state, BrowserLeaseState::Ready);

        timeout(
            Duration::from_secs(2),
            wait_for_count(&controller.shutdown_count, &controller.shutdowns, 1),
        )
        .await
        .unwrap();
        assert!(manager.list().await.is_empty());
    }

    #[tokio::test]
    async fn list_is_sorted_and_contains_no_kernel_identity() {
        let controller = FakeController::new();
        let (factory, _) = fake_factory({
            let controller = controller.clone();
            move |_| FakeKernel::new(controller.clone())
        });
        let manager = manager(factory, Duration::from_secs(1));
        manager
            .acquire("zeta", Duration::from_secs(5))
            .await
            .unwrap();
        manager
            .acquire("alpha", Duration::from_secs(6))
            .await
            .unwrap();

        let snapshots = manager.list().await;
        assert_eq!(
            snapshots
                .iter()
                .map(|snapshot| snapshot.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "zeta"]
        );
        assert!(snapshots
            .iter()
            .all(|snapshot| snapshot.state == BrowserLeaseState::Ready));
        assert!(snapshots
            .iter()
            .all(|snapshot| snapshot.remaining_idle.is_some()));
        let debug = format!("{snapshots:?}");
        assert!(!debug.contains("session_id"));
        assert!(!debug.contains("turn_id"));
        manager.release("alpha").await.unwrap();
        manager.release("zeta").await.unwrap();
    }

    #[tokio::test]
    async fn closed_ready_kernel_is_removed_before_reacquire() {
        let controller = FakeController::new();
        let closed = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let factory: KernelFactory = {
            let controller = controller.clone();
            let closed = closed.clone();
            let calls = calls.clone();
            Arc::new(move |_, _| {
                let index = calls.fetch_add(1, Ordering::SeqCst);
                let kernel_closed = if index == 0 {
                    closed.clone()
                } else {
                    Arc::new(AtomicBool::new(false))
                };
                let mut kernel = FakeKernel::new(controller.clone());
                kernel.closed = kernel_closed;
                Box::pin(async move { Ok(ManagedKernel::Fake(kernel)) })
            })
        };
        let manager = manager(factory, Duration::from_secs(1));
        manager
            .acquire("health", Duration::from_secs(1))
            .await
            .unwrap();
        closed.store(true, Ordering::SeqCst);
        manager
            .acquire("health", Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(manager.list().await.len(), 1);
        manager.release("health").await.unwrap();
    }
}
