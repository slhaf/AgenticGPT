//! One private Vaultwarden owner. CLI-native encrypted state is the only vault cache.
mod cli;
mod usage;
use crate::{
    config::{Config, RoomSecretConfig, SecretPolicy, ToolNamespace},
    private_state::PrivateStatePaths,
    state::AppState,
};
use agentic_gpt_protocol::{
    EventInjectRequest, EventSeverity, RoomSecretDeleteRequest, RoomSecretGetRequest,
    RoomSecretListField, RoomSecretListRequest, RoomSecretType, RoomSecretUpsertRequest,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::Utc;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::sync::{watch, Mutex};

type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Copy)]
enum Error {
    NotConfigured,
    Auth,
    Locked,
    NotFound,
    PresenceUnknown,
    ValueMissing,
    Ambiguous,
    InvalidTotp,
    Unavailable,
    Network,
    ConfirmationRejected,
    ConfirmationUnavailable,
    WriteUnknown,
    TooLarge,
    InvalidArguments,
}
impl Error {
    fn code(self) -> &'static str {
        match self {
            Self::NotConfigured => "secret_not_configured",
            Self::Auth => "secret_authentication_failed",
            Self::Locked => "secret_vault_locked",
            Self::NotFound => "secret_not_found",
            Self::PresenceUnknown => "secret_presence_unknown",
            Self::ValueMissing => "secret_value_missing",
            Self::Ambiguous => "ambiguous_secret",
            Self::InvalidTotp => "secret_invalid_totp",
            Self::Unavailable | Self::Network => "secret_provider_unavailable",
            Self::ConfirmationRejected => "secret_confirmation_rejected",
            Self::ConfirmationUnavailable => "secret_confirmation_unavailable",
            Self::WriteUnknown => "secret_write_outcome_unknown",
            Self::TooLarge => "secret_result_too_large",
            Self::InvalidArguments => "invalid_arguments",
        }
    }
    fn value(self) -> Value {
        let code = self.code();
        json!({"error":{"code":code,"message":code}})
    }
}
struct Owner {
    identity: String,
    cli: Arc<cli::Cli>,
    usage: usage::Usage,
    session: Mutex<Option<String>>,
    initialization: Mutex<()>,
    mutation: Mutex<()>,
    writes: Mutex<()>,
    _lease: crate::instance_lock::InstanceLock,
}
struct SyncJob {
    result: watch::Receiver<Option<Result<()>>>,
    handle: tokio::task::JoinHandle<()>,
}
pub(crate) struct SecretService {
    root: PathBuf,
    admission: Arc<tokio::sync::RwLock<()>>,
    reconfiguration: Mutex<()>,
    owner: parking_lot::Mutex<Option<Arc<Owner>>>,
    job: Mutex<Option<SyncJob>>,
    status: parking_lot::Mutex<Value>,
    stopped: Arc<std::sync::atomic::AtomicBool>,
    stopping: parking_lot::Mutex<tokio_util::sync::CancellationToken>,
}

/// Keeps new calls excluded until the caller publishes the new configuration.
pub(crate) struct SecretReconfigureGuard<'a> {
    service: &'a SecretService,
    _admission: tokio::sync::OwnedRwLockWriteGuard<()>,
    _reconfiguration: tokio::sync::MutexGuard<'a, ()>,
}

impl Drop for SecretReconfigureGuard<'_> {
    fn drop(&mut self) {
        self.service
            .stopped
            .store(false, std::sync::atomic::Ordering::Release);
    }
}
impl SecretService {
    pub(crate) fn open(paths: &PrivateStatePaths) -> anyhow::Result<Arc<Self>> {
        let root = paths.root.join("secret");
        cli::private_dir(&root)?;
        Ok(Arc::new(Self {
            root,
            admission: Arc::new(tokio::sync::RwLock::new(())),
            reconfiguration: Mutex::new(()),
            owner: parking_lot::Mutex::new(None),
            job: Mutex::new(None),
            status: parking_lot::Mutex::new(json!({"syncStatus":null,"syncLastTriggeredAt":null})),
            stopped: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            stopping: parking_lot::Mutex::new(tokio_util::sync::CancellationToken::new()),
        }))
    }
    pub(crate) fn info(&self) -> Value {
        self.status.lock().clone()
    }
    fn activate(&self, config: &RoomSecretConfig) -> Result<Arc<Owner>> {
        if self.stopped.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Error::Unavailable);
        }
        let identity = format!(
            "{:x}",
            Sha256::digest(
                format!(
                    "{}\0{}",
                    config.server_url.trim_end_matches('/'),
                    config.account.trim()
                )
                .as_bytes()
            )
        );
        let mut slot = self.owner.lock();
        if let Some(owner) = slot.as_ref() {
            return if owner.identity == identity {
                Ok(owner.clone())
            } else {
                Err(Error::NotConfigured)
            };
        }
        let root = self.root.join(&identity);
        cli::private_dir(&root).map_err(|_| Error::Unavailable)?;
        for directory in ["home", "tmp"] {
            cli::private_dir(&root.join(directory)).map_err(|_| Error::Unavailable)?;
        }
        let lease = crate::instance_lock::InstanceLock::acquire(
            &root.join("vault"),
            ".owner.lock",
            "secret",
        )
        .map_err(|_| Error::Unavailable)?;
        let owner = Arc::new(Owner {
            identity,
            usage: usage::Usage::open(root.join("usage.json")),
            cli: Arc::new(cli::Cli::new(root)),
            session: Mutex::new(None),
            initialization: Mutex::new(()),
            mutation: Mutex::new(()),
            writes: Mutex::new(()),
            _lease: lease,
        });
        *slot = Some(owner.clone());
        Ok(owner)
    }
    async fn trigger(
        self: &Arc<Self>,
        state: &AppState,
        owner: Arc<Owner>,
        config: RoomSecretConfig,
    ) -> watch::Receiver<Option<Result<()>>> {
        let mut slot = self.job.lock().await;
        if let Some(job) = slot.as_ref() {
            if job.result.borrow().is_none() {
                return job.result.clone();
            }
        }
        if let Some(job) = slot.take() {
            let _ = job.handle.await;
        }
        let (sender, receiver) = watch::channel(None);
        let triggered = Utc::now().to_rfc3339();
        *self.status.lock() = json!({"syncStatus":"syncing","syncLastTriggeredAt":triggered});
        let service = self.clone();
        let event_store = state.event_store.clone();
        let ttl = state.config.read().await.events.low_ttl_seconds;
        let stopping = self.stopping.lock().clone();
        let handle = tokio::spawn(async move {
            let synchronize = async {
                let _lease = owner.mutation.lock().await;
                owner.ensure_ready(&config).await?;
                owner.command(&["sync"], None, false, true).await?;
                // Prune only after a complete successful sync and complete local inventory.
                if let Ok(items) = owner.items().await {
                    owner
                        .usage
                        .prune(&items.iter().map(|i| i.id.clone()).collect());
                }
                Ok(())
            };
            let result: Result<()> = tokio::select! { result=synchronize=>result, _=stopping.cancelled()=>Err(Error::Unavailable) };
            service.status.lock()["syncStatus"] = json!(if result.is_ok() {
                "succeeded"
            } else {
                "failed"
            });
            let message = match result {
                Ok(()) => "密钥同步成功".to_owned(),
                Err(error) => format!("密钥同步失败：{}", error.code()),
            };
            let id = format!("secret-sync:{}", uuid::Uuid::new_v4());
            let _ = event_store.inject_external_once(
                &id,
                &EventInjectRequest {
                    message,
                    severity: Some(EventSeverity::Low),
                    reference: id.clone(),
                },
                ttl,
            );
            let _ = sender.send(Some(result));
        });
        *slot = Some(SyncJob {
            result: receiver.clone(),
            handle,
        });
        receiver
    }
    pub(crate) async fn shutdown(&self) {
        let _reconfiguration = self.reconfiguration.lock().await;
        if self.stopped.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        self.stop_commands().await;
        let _admission = self.admission.clone().write_owned().await;
        self.release_owner().await;
    }

    pub(crate) async fn prepare_reconfigure(&self) -> anyhow::Result<SecretReconfigureGuard<'_>> {
        let reconfiguration = self
            .reconfiguration
            .try_lock()
            .map_err(|_| anyhow::anyhow!("secret_reconfigure_busy"))?;
        // A busy request must leave the current owner usable. Do not cancel its
        // CLI until exclusive admission proves all request workflows finished.
        let admission = tokio::time::timeout(
            Duration::from_secs(30),
            self.admission.clone().write_owned(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("secret_reconfigure_busy"))?;
        self.stopped
            .store(true, std::sync::atomic::Ordering::Release);
        self.stop_commands().await;
        self.release_owner().await;
        *self.stopping.lock() = tokio_util::sync::CancellationToken::new();
        *self.status.lock() = json!({"syncStatus":null,"syncLastTriggeredAt":null});
        Ok(SecretReconfigureGuard {
            service: self,
            _admission: admission,
            _reconfiguration: reconfiguration,
        })
    }

    async fn stop_commands(&self) {
        self.stopping.lock().cancel();
        let owner = self.owner.lock().clone();
        if let Some(owner) = owner {
            owner.cli.stop().await;
        }
    }

    async fn release_owner(&self) {
        // Admission is exclusive: no request can retain an owner or start a job.
        if let Some(job) = self.job.lock().await.take() {
            let _ = job.handle.await;
        }
        let owner = self.owner.lock().take();
        if let Some(owner) = owner {
            let session = owner.session.lock().await.take();
            let mut env = BTreeMap::new();
            if let Some(session) = session {
                env.insert("BW_SESSION".to_owned(), session);
            }
            // Cancellation has reaped every normal command. A final private lock is bounded.
            let final_cli = Arc::new(cli::Cli::new(owner.cli.root.clone()));
            #[cfg(test)]
            {
                *final_cli.binary.write() = owner.cli.binary.read().clone();
            }
            let _ = final_cli.run(&["lock"], env, None, false, false).await;
            final_cli.stop().await;
        }
    }
}
impl Owner {
    async fn environment(&self) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        if let Some(session) = self.session.lock().await.as_ref() {
            env.insert("BW_SESSION".to_owned(), session.clone());
        }
        env
    }
    async fn command(
        &self,
        args: &[&str],
        input: Option<Vec<u8>>,
        write: bool,
        sync: bool,
    ) -> Result<Vec<u8>> {
        self.cli
            .run(args, self.environment().await, input, write, sync)
            .await
    }
    async fn status(&self) -> Result<Value> {
        cli::json(&self.command(&["status"], None, false, false).await?)
    }
    async fn ensure_ready(&self, config: &RoomSecretConfig) -> Result<()> {
        let _initializing = self.initialization.lock().await;
        let mut status = self.status().await?;
        if status["status"] == "unauthenticated" {
            self.command(
                &["config", "server", &config.server_url],
                None,
                false,
                false,
            )
            .await?;
            let mut env = BTreeMap::new();
            env.insert("BW_CLIENTID".to_owned(), credential(&config.client_id)?);
            env.insert(
                "BW_CLIENTSECRET".to_owned(),
                credential(&config.client_secret)?,
            );
            self.cli
                .run(&["login", "--apikey"], env, None, false, false)
                .await?;
            status = self.status().await?;
        }
        verify_identity(&status, config)?;
        if status["status"] != "unlocked" {
            let mut env = BTreeMap::new();
            env.insert(
                "AGENTIC_BW_PASSWORD".to_owned(),
                credential(&config.master_password)?,
            );
            let bytes = self
                .cli
                .run(
                    &["unlock", "--passwordenv", "AGENTIC_BW_PASSWORD", "--raw"],
                    env,
                    None,
                    false,
                    false,
                )
                .await?;
            let session = String::from_utf8(bytes).map_err(|_| Error::Locked)?;
            let session = session.trim();
            if session.is_empty() || session.len() > 8192 || session.chars().any(char::is_control) {
                return Err(Error::Locked);
            }
            *self.session.lock().await = Some(session.to_owned());
            status = self.status().await?;
            verify_identity(&status, config)?;
            if status["status"] != "unlocked" {
                return Err(Error::Locked);
            }
        }
        Ok(())
    }
    async fn items(&self) -> Result<Vec<Item>> {
        let value = cli::json(&self.command(&["list", "items"], None, false, false).await?)?;
        let array = value.as_array().ok_or(Error::Unavailable)?;
        let mut items = Vec::new();
        for value in array {
            if value["type"] != 1 || !value["deletedDate"].is_null() {
                continue;
            }
            let site = value["name"].as_str().ok_or(Error::Unavailable)?.trim();
            let account = value["login"]["username"].as_str().unwrap_or("").trim();
            // Non-account Login entries cannot form a public Secret identity.
            if site.is_empty() || account.is_empty() {
                continue;
            }
            let id = value["id"].as_str().ok_or(Error::Unavailable)?;
            uuid::Uuid::parse_str(id).map_err(|_| Error::Unavailable)?;
            items.push(Item {
                id: id.to_owned(),
                site: site.to_owned(),
                account: account.to_owned(),
            });
        }
        Ok(items)
    }
}
struct Item {
    id: String,
    site: String,
    account: String,
}
fn credential(reference: &str) -> Result<String> {
    if !(reference.starts_with("env:") || reference.starts_with("file:")) {
        return Err(Error::NotConfigured);
    }
    crate::config::resolve_secret_reference(reference).map_err(|_| Error::NotConfigured)
}
fn verify_identity(status: &Value, config: &RoomSecretConfig) -> Result<()> {
    let actual = status["serverUrl"].as_str().ok_or(Error::Auth)?;
    let actual = url::Url::parse(actual).map_err(|_| Error::Auth)?;
    let expected = url::Url::parse(&config.server_url).map_err(|_| Error::NotConfigured)?;
    if actual.as_str().trim_end_matches('/') != expected.as_str().trim_end_matches('/')
        || status["userEmail"].as_str().map(str::trim) != Some(config.account.trim())
    {
        return Err(Error::Auth);
    }
    Ok(())
}
async fn await_sync(mut receiver: watch::Receiver<Option<Result<()>>>) -> Result<()> {
    loop {
        if let Some(result) = *receiver.borrow_and_update() {
            return result;
        }
        receiver.changed().await.map_err(|_| Error::Unavailable)?;
    }
}
fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|_| Error::InvalidArguments)
}
fn identity(site: &str, account: &str) -> Result<(String, String)> {
    let site = site.trim();
    let account = account.trim();
    if site.len() > 1024
        || account.len() > 1024
        || site.trim().is_empty()
        || account.trim().is_empty()
        || site.chars().any(char::is_control)
        || account.chars().any(char::is_control)
    {
        return Err(Error::InvalidArguments);
    }
    Ok((site.trim().to_owned(), account.trim().to_owned()))
}
fn locate<'a>(items: &'a [Item], site: &str, account: &str) -> Result<Option<&'a Item>> {
    let mut matches = items
        .iter()
        .filter(|item| item.site.to_lowercase() == site.to_lowercase() && item.account == account);
    let first = matches.next();
    if matches.next().is_some() {
        return Err(Error::Ambiguous);
    }
    Ok(first)
}
async fn confirm(
    state: &AppState,
    config: &Config,
    policy: SecretPolicy,
    operation: &str,
    site: &str,
    account: &str,
    fields: &str,
) -> Result<()> {
    if policy == SecretPolicy::Allow {
        return Ok(());
    }
    let preview = format!("Site: {site}\nAccount: {account}\nFields: {fields}");
    crate::confirmation::authorize_sensitive_operation(
        state,
        config,
        crate::policy::PolicyDecision::Confirm,
        operation,
        "Confirm secret access",
        &preview,
        state.secret.stopped.clone(),
    )
    .await
    .map_err(|reason| {
        if matches!(
            reason.as_str(),
            "deny" | "deny_once" | "denied" | "reject" | "rejected"
        ) {
            Error::ConfirmationRejected
        } else {
            Error::ConfirmationUnavailable
        }
    })?;
    Ok(())
}

pub(crate) async fn dispatch(state: &AppState, operation: &str, args: Value) -> Value {
    let admission = match state.secret.admission.clone().try_read_owned() {
        Ok(guard) => guard,
        Err(_) => return Error::Unavailable.value(),
    };
    if state
        .secret
        .stopped
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Error::Unavailable.value();
    }
    let config = state.config.read().await.clone();
    if !config.toolsets.is_enabled(ToolNamespace::RoomSecret) {
        return json!({"error":{"code":"toolset_required","message":"operation requires toolset room.secret to be enabled"}});
    }
    let secret = &config.room.secret;
    if secret.server_url.is_empty() || secret.account.is_empty() {
        return Error::NotConfigured.value();
    }
    let write = matches!(operation, "room.secret.upsert" | "room.secret.delete");
    let state = state.clone();
    let operation = operation.to_owned();
    // Reads also own admission until their CLI children have been reaped, even
    // when the ingress stops waiting for the result.
    tokio::spawn(async move {
        let _admission = admission;
        dispatch_inner(&state, &config, &operation, args, write).await
    })
    .await
    .unwrap_or_else(|_| {
        if write {
            Error::WriteUnknown
        } else {
            Error::Unavailable
        }
        .value()
    })
}

async fn dispatch_inner(
    state: &AppState,
    config: &Config,
    operation: &str,
    args: Value,
    write: bool,
) -> Value {
    let secret = &config.room.secret;
    let owner = match state.secret.activate(secret) {
        Ok(owner) => owner,
        Err(error) => return error.value(),
    };
    let _write = if write {
        Some(owner.writes.lock().await)
    } else {
        None
    };
    if state
        .secret
        .stopped
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Error::Unavailable.value();
    }
    let receiver = state
        .secret
        .trigger(state, owner.clone(), secret.clone())
        .await;
    let stale = if write {
        if let Err(error) = await_sync(receiver).await {
            return error.value();
        }
        false
    } else {
        match tokio::time::timeout(
            Duration::from_millis(secret.sync_wait_timeout_ms),
            await_sync(receiver),
        )
        .await
        {
            Ok(Ok(())) => false,
            Ok(Err(Error::Network)) | Err(_) => true,
            Ok(Err(error)) => return error.value(),
        }
    };
    // Reads deliberately do not take the synchronization/mutation lock.
    let _mutation = if write {
        Some(owner.mutation.lock().await)
    } else {
        None
    };
    let freshness = if stale {
        let status = match owner.status().await {
            Ok(status) => status,
            Err(error) => return error.value(),
        };
        let Some(last) = status.get("lastSync") else {
            return Error::Unavailable.value();
        };
        if !last.is_null()
            && !last
                .as_str()
                .is_some_and(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok())
        {
            return Error::Unavailable.value();
        }
        Some(last.clone())
    } else {
        None
    };
    let result = execute(state, config, &owner, operation, args, stale).await;
    let mut value = match result {
        Ok(value) => value,
        Err(error) => error.value(),
    };
    if let Some(last) = freshness {
        value["stale"] = json!(true);
        value["lastSyncedAt"] = last;
    }

    value
}
async fn execute(
    state: &AppState,
    config: &Config,
    owner: &Owner,
    operation: &str,
    args: Value,
    stale: bool,
) -> Result<Value> {
    // ensure_ready coordinates login/unlock, and retries that explicit state transition once.
    owner.ensure_ready(&config.room.secret).await?;
    let items = owner.items().await?;
    match operation {
        "room.secret.list" => {
            let request: RoomSecretListRequest = parse(args)?;
            if request.filter.as_ref().is_some_and(|s| s.len() > 1024) {
                return Err(Error::InvalidArguments);
            }
            let filter = request.filter.map(|s| s.to_lowercase());
            let mut selected: Vec<_> = items
                .iter()
                .filter_map(|item| {
                    let quality = if let Some(filter) = filter.as_ref() {
                        let site = item.site.to_lowercase();
                        let account = item.account.to_lowercase();
                        let values: Vec<&str> = match request.field {
                            Some(RoomSecretListField::Site) => vec![&site],
                            Some(RoomSecretListField::Account) => vec![&account],
                            _ => vec![&site, &account],
                        };
                        values
                            .into_iter()
                            .filter(|value| value.contains(filter))
                            .map(|value| {
                                if value == filter {
                                    0
                                } else if value.starts_with(filter) {
                                    1
                                } else {
                                    2
                                }
                            })
                            .min()?
                    } else {
                        0
                    };
                    Some((item, quality, owner.usage.rank(&item.id)))
                })
                .collect();
            selected.sort_by(|a, b| {
                a.1.cmp(&b.1)
                    .then_with(|| b.2 .0.total_cmp(&a.2 .0))
                    .then_with(|| b.2 .1.cmp(&a.2 .1))
                    .then_with(|| a.0.site.cmp(&b.0.site))
                    .then_with(|| a.0.account.cmp(&b.0.account))
            });
            let value = json!({"items":selected.into_iter().map(|(item,_,_)|json!({"site":item.site,"account":item.account})).collect::<Vec<_>>()});
            if serde_json::to_vec(&value)
                .map_err(|_| Error::Unavailable)?
                .len()
                > 512 * 1024
            {
                return Err(Error::TooLarge);
            }
            Ok(value)
        }
        "room.secret.get" => {
            let request: RoomSecretGetRequest = parse(args)?;
            let (site, account) = identity(&request.site, &request.account)?;
            let item = locate(&items, &site, &account)?.ok_or(if stale {
                Error::PresenceUnknown
            } else {
                Error::NotFound
            })?;
            confirm(
                state,
                config,
                config.room.secret.get_policy,
                operation,
                &site,
                &account,
                match request.secret_type {
                    RoomSecretType::Password => "password",
                    RoomSecretType::Totp => "totp",
                },
            )
            .await?;
            let value = cli::json(
                &owner
                    .command(&["get", "item", &item.id], None, false, false)
                    .await?,
            )?;
            let field = if request.secret_type == RoomSecretType::Password {
                "password"
            } else {
                "totp"
            };
            if !value["login"][field]
                .as_str()
                .is_some_and(|s| !s.is_empty())
            {
                return Err(if stale {
                    Error::PresenceUnknown
                } else {
                    Error::ValueMissing
                });
            }
            let result = if request.secret_type == RoomSecretType::Password {
                value["login"]["password"]
                    .as_str()
                    .ok_or(Error::Unavailable)?
                    .to_owned()
            } else {
                validate_totp(value["login"]["totp"].as_str().ok_or(Error::InvalidTotp)?)?;
                let bytes = owner
                    .command(&["get", "totp", &item.id], None, false, false)
                    .await?;
                let code = std::str::from_utf8(&bytes)
                    .map_err(|_| Error::InvalidTotp)?
                    .trim();
                if !(code.len() == 6 || code.len() == 8)
                    || !code.bytes().all(|b| b.is_ascii_digit())
                {
                    return Err(Error::InvalidTotp);
                }
                code.to_owned()
            };
            if result.len() > 1024 * 1024 {
                return Err(Error::TooLarge);
            }
            owner.usage.used(&item.id);
            Ok(json!({"value":result}))
        }
        "room.secret.upsert" => {
            // Explicit null is not an omitted field and must never clear data.
            let request: RoomSecretUpsertRequest = parse(args)?;
            let (site, account) = identity(&request.site, &request.account)?;
            if request.password.is_none() && request.totp.is_none() {
                return Err(Error::InvalidArguments);
            }
            for value in [&request.password, &request.totp].into_iter().flatten() {
                if value.is_empty() || value.len() > 64 * 1024 {
                    return Err(Error::InvalidArguments);
                }
            }
            if let Some(totp) = &request.totp {
                validate_totp(totp)?;
            }
            let item = locate(&items, &site, &account)?;
            let fields = match (request.password.is_some(), request.totp.is_some()) {
                (true, true) => "password, totp",
                (true, false) => "password",
                _ => "totp",
            };
            confirm(
                state,
                config,
                config.room.secret.upsert_policy,
                operation,
                &site,
                &account,
                fields,
            )
            .await?;
            let mut value = if let Some(item) = item {
                cli::json(
                    &owner
                        .command(&["get", "item", &item.id], None, false, false)
                        .await?,
                )?
            } else {
                json!({"type":1,"name":site,"login":{"username":account},"favorite":false,"reprompt":0,"notes":null,"fields":[],"secureNote":null,"card":null,"identity":null})
            };
            if !value.is_object() || !value["login"].is_object() {
                return Err(Error::Unavailable);
            }
            if let Some(password) = request.password {
                value["login"]["password"] = json!(password);
            }
            if let Some(totp) = request.totp {
                value["login"]["totp"] = json!(totp);
            }
            let input = STANDARD
                .encode(serde_json::to_vec(&value).map_err(|_| Error::Unavailable)?)
                .into_bytes();
            let args = if let Some(item) = item {
                vec!["edit", "item", item.id.as_str()]
            } else {
                vec!["create", "item"]
            };
            let bytes = owner.command(&args, Some(input), true, false).await?;
            let response = cli::json(&bytes).map_err(|_| Error::WriteUnknown)?;
            let id = response["id"].as_str().ok_or(Error::WriteUnknown)?;
            uuid::Uuid::parse_str(id).map_err(|_| Error::WriteUnknown)?;
            if let Some(item) = item {
                if item.id != id {
                    return Err(Error::WriteUnknown);
                }
            }
            Ok(if item.is_some() {
                json!({"updated":true})
            } else {
                json!({"created":true})
            })
        }
        "room.secret.delete" => {
            let request: RoomSecretDeleteRequest = parse(args)?;
            let (site, account) = identity(&request.site, &request.account)?;
            let item = locate(&items, &site, &account)?.ok_or(Error::NotFound)?;
            confirm(
                state,
                config,
                config.room.secret.delete_policy,
                operation,
                &site,
                &account,
                "password, totp",
            )
            .await?;
            owner
                .command(&["delete", "item", &item.id], None, true, false)
                .await?;
            owner.usage.remove(&item.id);
            Ok(json!({"deleted":true}))
        }
        _ => Err(Error::InvalidArguments),
    }
}
fn validate_totp(value: &str) -> Result<()> {
    fn base32(value: &str) -> bool {
        let value = value.trim_end_matches('=');
        value.len() >= 16
            && value.bytes().all(|b| {
                b.is_ascii_uppercase() || b.is_ascii_lowercase() || matches!(b, b'2'..=b'7')
            })
            && matches!(value.len() % 8, 0 | 2 | 4 | 5 | 7)
    }
    if !value.starts_with("otpauth:") {
        return if base32(value) {
            Ok(())
        } else {
            Err(Error::InvalidTotp)
        };
    }
    let uri = url::Url::parse(value).map_err(|_| Error::InvalidTotp)?;
    if uri.scheme() != "otpauth"
        || uri.host_str() != Some("totp")
        || uri.path().len() <= 1
        || uri.fragment().is_some()
        || uri.port().is_some()
        || !uri.username().is_empty()
        || uri.password().is_some()
    {
        return Err(Error::InvalidTotp);
    }
    let mut seen = BTreeSet::new();
    let mut secret = false;
    for (key, value) in uri.query_pairs() {
        if !seen.insert(key.to_string()) {
            return Err(Error::InvalidTotp);
        }
        match key.as_ref() {
            "secret" => {
                if !base32(&value) {
                    return Err(Error::InvalidTotp);
                }
                secret = true;
            }
            "algorithm" => {
                if !matches!(
                    value.to_ascii_uppercase().as_str(),
                    "SHA1" | "SHA256" | "SHA512"
                ) {
                    return Err(Error::InvalidTotp);
                }
            }
            "digits" => {
                if !matches!(value.as_ref(), "6" | "8") {
                    return Err(Error::InvalidTotp);
                }
            }
            "period" => {
                if value != "30" {
                    return Err(Error::InvalidTotp);
                }
            }
            "issuer" => {}
            _ => return Err(Error::InvalidTotp),
        }
    }
    if secret {
        Ok(())
    } else {
        Err(Error::InvalidTotp)
    }
}

#[cfg(test)]
mod tests;
