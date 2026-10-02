// Tests for the crate entrypoint, startup, discovery, and CLI composition.
use super::*;
use crate::browser_discovery::{
    resolve_browser_runtime, resolve_browser_runtime_with_sources, BrowserRuntimeSources,
};
use crate::cli::{read_local_arguments, Cli, Commands, LocalCommand, MAX_LOCAL_ARGUMENT_BYTES};
use crate::config::mcp_servers::McpServerConfig;
use crate::config::{Config, RuntimeMode};
use crate::config::{PathPolicyConfig, Rule, TunnelConfig};
use crate::config_cli::{PathRootCommand, PathRootKind};
use crate::exec::PreparedBatchElement;
use crate::policy::PolicyDecision;
use crate::startup::{apply_live_config_subset, build_app_state, reload_live_config_once};
use crate::state::{AppState, BrowserRuntimeContext, CapabilityProfile, RuntimeModel};
use agentic_gpt_protocol::{
    AgentMessage, BootstrapReadRequest, HubCommand, RoomDiaryActiveRequest,
};
use anyhow::anyhow;
use clap::Parser;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

fn explicit_browser_runtime_config() -> config::ExplicitBrowserRuntimeConfig {
    config::ExplicitBrowserRuntimeConfig {
        app_version: Some("26.1.2".to_string()),
        channel: Some("prod".to_string()),
        node_repl_path: Some("/opt/runtime/node_repl".to_string()),
        node_path: Some("/opt/runtime/node".to_string()),
        browser_client_path: Some("/opt/runtime/chrome/scripts/browser-client.mjs".to_string()),
        browser_service_path: Some("/opt/runtime/chrome/scripts/browser-service.mjs".to_string()),
        codex_home: Some("/opt/runtime/home".to_string()),
        codex_cli_path: Some("/opt/runtime/codex".to_string()),
        node_module_dirs: Vec::new(),
    }
}

#[tokio::test]
async fn explicit_runtime_precedes_desktop_discovery_and_invalid_does_not_fallback() {
    let mut config = Config::default_config().unwrap();
    config.browser.runtime = Some(explicit_browser_runtime_config());
    let context = resolve_browser_runtime(&config)
        .await
        .expect("explicit runtime selected");
    assert_eq!(context.descriptor.app_version, "26.1.2");
    config.browser.runtime.as_mut().unwrap().node_path = Some("relative/node".to_string());
    assert!(resolve_browser_runtime(&config).await.is_none());
}

#[test]
fn invalid_explicit_runtime_keeps_app_startup_fail_open() {
    let mut config = Config::default_config().unwrap();
    config.browser.runtime = Some(explicit_browser_runtime_config());
    config.browser.runtime.as_mut().unwrap().node_path = Some("relative/node".to_string());
    let root = unique_temp_dir("explicit-browser-fail-open");
    config.workspace_root = root.join("workspace");
    let state = build_app_state(
        root.join("config.json"),
        config,
        RuntimeModel::local(CapabilityProfile::Normal),
        false,
        None,
    )
    .unwrap();
    assert!(state.browser_runtime.is_none());
    let _ = fs::remove_dir_all(root);
}

fn managed_config(auto_provision: bool) -> Config {
    let mut config = Config::default_config().unwrap();
    config.browser.runtime = None;
    config.browser.managed.enabled = true;
    config.browser.managed.auto_provision = auto_provision;
    config
}

fn test_browser_descriptor(
    label: &str,
    codex_home: &Path,
    codex_cli_path: Option<&str>,
) -> browser_runtime::BrowserRuntimeDescriptor {
    let root = PathBuf::from(format!("/tmp/agentic-browser-source-{label}"));
    browser_runtime::BrowserRuntimeDescriptor {
        app_version: format!("test-{label}"),
        channel: "prod".to_string(),
        node_repl_path: root.join("cua_node/bin/node_repl"),
        node_path: root.join("cua_node/bin/node"),
        browser_client_path: root.join("chrome/scripts/browser-client.mjs"),
        browser_service_path: root.join("chrome/scripts/browser-service.mjs"),
        codex_home: codex_home.to_path_buf(),
        codex_cli_path: codex_cli_path.map(PathBuf::from),
        node_module_dirs: Vec::new(),
        trusted_code_paths: vec![codex_home.to_path_buf()],
        docs_root: root.join("chrome/docs"),
    }
}

fn injected_browser_sources(
    cache_root: PathBuf,
    codex_home: PathBuf,
    cache: Option<browser_runtime::BrowserRuntimeDescriptor>,
    provision: Option<browser_runtime::BrowserRuntimeDescriptor>,
    desktop: Option<browser_runtime::BrowserRuntimeDescriptor>,
    provision_calls: Arc<AtomicUsize>,
    desktop_calls: Arc<AtomicUsize>,
) -> BrowserRuntimeSources {
    let cache = Arc::new(cache);
    let provision = Arc::new(provision);
    let desktop = Arc::new(desktop);
    BrowserRuntimeSources {
        managed_cache_root: Arc::new(move || Ok(cache_root.clone())),
        managed_codex_home: Arc::new(move || Ok(codex_home.clone())),
        managed_target: Arc::new(|| Ok("test-target")),
        managed_discover: Arc::new(move |_, _, _| {
            cache
                .as_ref()
                .clone()
                .ok_or_else(|| anyhow!("test_managed_cache_miss"))
        }),
        managed_provision: Arc::new(move |_| {
            provision_calls.fetch_add(1, Ordering::SeqCst);
            let provision = provision.clone();
            Box::pin(async move {
                provision
                    .as_ref()
                    .clone()
                    .ok_or_else(|| anyhow!("test_managed_provision_failed"))
            })
        }),
        desktop_registry_path: Arc::new(|| Ok(PathBuf::from("/tmp/test-desktop-registry.json"))),
        desktop_discover: Arc::new(move |_| {
            desktop_calls.fetch_add(1, Ordering::SeqCst);
            desktop
                .as_ref()
                .clone()
                .ok_or_else(|| anyhow!("test_desktop_unavailable"))
        }),
    }
}

#[tokio::test]
async fn managed_cache_precedes_desktop_and_preserves_descriptor_fields() {
    let root = unique_temp_dir("browser-source-cache");
    let codex_home = root.join("browser-runtime").join("codex-home");
    let provision_calls = Arc::new(AtomicUsize::new(0));
    let desktop_calls = Arc::new(AtomicUsize::new(0));
    let cached = test_browser_descriptor("cache", &codex_home, None);
    let desktop = test_browser_descriptor("desktop", &codex_home, Some("/tmp/desktop-cli"));
    let sources = injected_browser_sources(
        root.clone(),
        codex_home.clone(),
        Some(cached),
        Some(test_browser_descriptor(
            "provision",
            &codex_home,
            Some("/tmp/provision-cli"),
        )),
        Some(desktop),
        provision_calls.clone(),
        desktop_calls.clone(),
    );

    let context = resolve_browser_runtime_with_sources(&managed_config(false), &sources)
        .await
        .expect("managed cache selected");
    assert_eq!(context.descriptor.app_version, "test-cache");
    assert_eq!(context.descriptor.channel, "prod");
    assert_eq!(context.descriptor.codex_home, codex_home);
    assert_eq!(context.descriptor.codex_cli_path, None);
    assert_eq!(provision_calls.load(Ordering::SeqCst), 0);
    assert_eq!(desktop_calls.load(Ordering::SeqCst), 0);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(context.descriptor.codex_home.clone())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn auto_provision_is_gated_and_failures_fall_back_to_desktop() {
    let root = unique_temp_dir("browser-source-provision");
    let codex_home = root.join("codex-home");
    let provision_calls = Arc::new(AtomicUsize::new(0));
    let desktop_calls = Arc::new(AtomicUsize::new(0));
    let provisioned = test_browser_descriptor("provision", &codex_home, None);
    let desktop = test_browser_descriptor("desktop", &codex_home, None);

    let enabled_sources = injected_browser_sources(
        root.clone(),
        codex_home.clone(),
        None,
        Some(provisioned),
        Some(desktop.clone()),
        provision_calls.clone(),
        desktop_calls.clone(),
    );
    let context = resolve_browser_runtime_with_sources(&managed_config(true), &enabled_sources)
        .await
        .expect("managed provision selected");
    assert_eq!(context.descriptor.app_version, "test-provision");
    assert_eq!(provision_calls.load(Ordering::SeqCst), 1);
    assert_eq!(desktop_calls.load(Ordering::SeqCst), 0);

    let disabled_sources = injected_browser_sources(
        root.clone(),
        codex_home.clone(),
        None,
        Some(test_browser_descriptor(
            "disabled-provision",
            &codex_home,
            None,
        )),
        Some(desktop),
        provision_calls.clone(),
        desktop_calls.clone(),
    );
    let context = resolve_browser_runtime_with_sources(&managed_config(false), &disabled_sources)
        .await
        .expect("desktop fallback selected");
    assert_eq!(context.descriptor.app_version, "test-desktop");
    assert_eq!(provision_calls.load(Ordering::SeqCst), 1);
    assert_eq!(desktop_calls.load(Ordering::SeqCst), 1);

    let failed_sources = injected_browser_sources(
        root.clone(),
        codex_home.clone(),
        None,
        None,
        Some(test_browser_descriptor("failed-desktop", &codex_home, None)),
        provision_calls.clone(),
        desktop_calls.clone(),
    );
    let context = resolve_browser_runtime_with_sources(&managed_config(true), &failed_sources)
        .await
        .expect("desktop fallback after provisioning failure");
    assert_eq!(context.descriptor.app_version, "test-failed-desktop");
    assert_eq!(provision_calls.load(Ordering::SeqCst), 2);
    assert_eq!(desktop_calls.load(Ordering::SeqCst), 2);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn unavailable_sources_fail_open_without_host_io() {
    let root = unique_temp_dir("browser-source-unavailable");
    let codex_home = root.join("codex-home");
    let provision_calls = Arc::new(AtomicUsize::new(0));
    let desktop_calls = Arc::new(AtomicUsize::new(0));
    let sources = injected_browser_sources(
        root.clone(),
        codex_home,
        None,
        None,
        None,
        provision_calls.clone(),
        desktop_calls.clone(),
    );
    assert!(
        resolve_browser_runtime_with_sources(&managed_config(true), &sources)
            .await
            .is_none()
    );
    assert_eq!(provision_calls.load(Ordering::SeqCst), 1);
    assert_eq!(desktop_calls.load(Ordering::SeqCst), 1);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn builder_accepts_pre_resolved_browser_context_without_discovery() {
    let root = unique_temp_dir("browser-builder-resolved");
    let mut config = Config::default_config().unwrap();
    config.workspace_root = root.join("workspace");
    let descriptor = test_browser_descriptor("builder", &root.join("codex-home"), None);
    let launch_spec =
        browser_runtime::build_node_repl_launch_spec(&descriptor, &BTreeMap::new()).unwrap();
    let context = BrowserRuntimeContext::new(descriptor, launch_spec);
    let state = build_app_state(
        root.join("config.json"),
        config,
        RuntimeModel::local(CapabilityProfile::Normal),
        false,
        Some(context),
    )
    .unwrap();
    assert_eq!(
        state
            .browser_runtime
            .as_ref()
            .unwrap()
            .descriptor
            .app_version,
        "test-builder"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cli_version_uses_crate_version() {
    let error = match Cli::try_parse_from(["agentic-gpt", "--version"]) {
        Ok(_) => panic!("--version unexpectedly parsed as a runnable command"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), clap::error::ErrorKind::DisplayVersion);
    let rendered = error.to_string();
    assert!(rendered.contains("agentic-gpt 0.9.1"));
    assert!(rendered.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn sse_post_status_classification_stops_on_stale_connection() {
    assert_eq!(
        hub::classify_sse_post_status(reqwest::StatusCode::OK),
        hub::SsePostStatus::Delivered
    );
    assert_eq!(
        hub::classify_sse_post_status(reqwest::StatusCode::CONFLICT),
        hub::SsePostStatus::Stale
    );
    assert_eq!(
        hub::classify_sse_post_status(reqwest::StatusCode::BAD_GATEWAY),
        hub::SsePostStatus::Retry
    );
}

#[test]
fn run_as_room_uses_workspace_default_repository_root() {
    let config = Config::default_config().unwrap();
    assert_eq!(config.agent_id, "laptop");
    assert_eq!(
        room_repository::repository_root(&config),
        config.workspace_root.join("room")
    );
}

#[test]
fn public_run_has_only_a_config_path_and_no_profile_override() {
    let cli = Cli::try_parse_from(["agentic-gpt", "run"]).unwrap();
    assert!(matches!(cli.command, Commands::Run { config: None }));
    assert!(Cli::try_parse_from(["agentic-gpt", "run", "--profile", "room"]).is_err());
}

#[test]
fn local_cli_accepts_config_before_or_after_subcommand() {
    for args in [
        vec![
            "agentic-gpt",
            "local",
            "--config",
            "/tmp/local.json",
            "list-tools",
        ],
        vec![
            "agentic-gpt",
            "local",
            "list-tools",
            "--config",
            "/tmp/local.json",
        ],
    ] {
        let cli = Cli::try_parse_from(args).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Local {
                config: Some(ref path),
                command: LocalCommand::ListTools,
            } if path == &PathBuf::from("/tmp/local.json")
        ));
    }
}

#[test]
fn local_arguments_are_bounded_objects_from_inline_or_file() {
    assert!(read_local_arguments(None, None).unwrap().is_empty());
    let inline = read_local_arguments(Some(r#"{"value":"ok"}"#.to_string()), None).unwrap();
    assert_eq!(inline["value"], "ok");

    let root = unique_temp_dir("local-arguments");
    let path = root.join("args.json");
    fs::write(&path, br#"{"fromFile":true}"#).unwrap();
    let from_file = read_local_arguments(None, Some(path.to_string_lossy().into_owned())).unwrap();
    assert_eq!(from_file["fromFile"], true);

    assert!(read_local_arguments(Some("[]".to_string()), None)
        .unwrap_err()
        .to_string()
        .starts_with("local_arguments_must_be_object"));
    assert!(read_local_arguments(Some("{".to_string()), None)
        .unwrap_err()
        .to_string()
        .starts_with("local_arguments_invalid_json"));
    assert!(
        read_local_arguments(Some("x".repeat(MAX_LOCAL_ARGUMENT_BYTES + 1)), None)
            .unwrap_err()
            .to_string()
            .starts_with("local_arguments_too_large")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn configured_room_repository_root_overrides_default() {
    let mut config = Config::default_config().unwrap();
    let root = unique_temp_dir("configured-room-repository");
    config.room.repository_root = Some(root.clone());
    assert_eq!(room_repository::repository_root(&config), root);
}

#[test]
fn room_timezone_defaults_and_can_be_overridden() {
    let mut config = Config::default_config().unwrap();
    assert_eq!(config.room.timezone, "Asia/Shanghai");
    assert_eq!(config.room.diary_day_boundary_hour, 5);
    config.room.timezone = "UTC".to_string();
    config.room.diary_day_boundary_hour = 3;
    assert_eq!(config.room.timezone, "UTC");
    assert_eq!(config.room.diary_day_boundary_hour, 3);
}

fn command_test_state(
    profile: CapabilityProfile,
    workspace_root: PathBuf,
) -> (AppState, mpsc::UnboundedReceiver<AgentMessage>) {
    let mut config = Config::default_config().unwrap();
    config.toolsets = if profile == CapabilityProfile::Room {
        config::ToolsetConfig::room()
    } else {
        config::ToolsetConfig::normal()
    };
    config.workspace_root = workspace_root;
    let (tx, rx) = mpsc::unbounded_channel();
    let private_state = crate::private_state::PrivateStatePaths::for_test_agent(
        std::env::temp_dir().join(format!(
            "agentic-test-private-{}",
            uuid::Uuid::new_v4().simple()
        )),
        config.agent_id.clone(),
    );
    let process_history = crate::process_history::ProcessHistoryStore::open(&private_state);
    (
        AppState {
            config_path: PathBuf::from("test-config.json"),
            config: Arc::new(RwLock::new(config)),
            event_store: crate::event_store::EventStore::open(&private_state).unwrap(),
            private_state,
            process_history,
            browser_runtime: None,
            runtime: RuntimeModel::hub(profile),
            started_at: chrono::Utc::now(),
            boot_generation: "testboot0001".to_string(),
            supervised: false,
            file_locks: Arc::new(Mutex::new(HashMap::new())),
            processes: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(Some(tx))),
            reporting_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            mcp_concurrency: Arc::new(crate::process::McpConcurrency::new()),
            room_repository_writes: Arc::new(Mutex::new(())),
            skills_writes: Arc::new(Mutex::new(())),
            skill_leases: Arc::new(skills::SkillLeaseManager::new()),
            skill_installs: Arc::new(skill_installs::InstallManager::new()),
        },
        rx,
    )
}

async fn recv_response(rx: &mut mpsc::UnboundedReceiver<AgentMessage>) -> serde_json::Value {
    let message = rx.recv().await.unwrap();
    let AgentMessage::Response { data, .. } = message else {
        panic!("expected agent response");
    };
    data
}

#[tokio::test]
async fn normal_runtime_follows_live_room_toolset_for_bootstrap_dispatch() {
    let workspace = unique_temp_dir("normal-live-room-bootstrap").join("workspace");
    let guides = workspace.join("bootstrap").join("guides");
    fs::create_dir_all(&guides).unwrap();
    fs::write(
            workspace.join("bootstrap").join("bootstrap.md"),
            "---\nid: room\nkind: entrypoint\nname: Room\ndescription: Route guides\nschemaVersion: 1\n---\nstart\n",
        )
        .unwrap();
    fs::write(
        guides.join("guide.md"),
        "---\nid: guide\nkind: guide\ntitle: Guide\nsummary: Use guide\n---\nbody\n",
    )
    .unwrap();
    let (state, _rx) = command_test_state(CapabilityProfile::Normal, workspace);

    let disabled = local_service::dispatch(
        state.clone(),
        HubCommand::RoomBootstrap {
            request_id: "req-disabled".to_string(),
        },
        operation::RequestContext::new(operation::RequestIngress::Hub, "room.bootstrap"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(disabled["error"]["code"], "room_toolset_required");

    let mut config = state.config.read().await.clone();
    config.toolsets.enable(config::ToolNamespace::Room);
    *state.config.write().await = config;

    let enabled = local_service::dispatch(
        state,
        HubCommand::RoomBootstrap {
            request_id: "req-enabled".to_string(),
        },
        operation::RequestContext::new(operation::RequestIngress::Hub, "room.bootstrap"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(enabled["entrypoint"]["id"], "room");
    assert_eq!(enabled["guides"][0]["id"], "guide");
}

#[tokio::test]
async fn hub_adapter_and_local_dispatcher_share_toolset_errors() {
    let workspace = unique_temp_dir("dispatcher-parity").join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let (state, mut rx) = command_test_state(CapabilityProfile::Normal, workspace);
    let command = HubCommand::RoomBootstrap {
        request_id: "req-parity".to_string(),
    };

    let direct = local_service::dispatch(
        state.clone(),
        command.clone(),
        operation::RequestContext::new(
            operation::RequestIngress::Hub,
            operation::hub_command_name(&command),
        ),
        None,
    )
    .await
    .unwrap();
    hub::handle_hub_command(state, command, None).await.unwrap();
    let adapted = recv_response(&mut rx).await;
    assert_eq!(direct["error"], adapted["error"]);
    assert_eq!(direct["error"]["code"], "room_toolset_required");
}

#[tokio::test]
async fn hub_panel_failure_emits_live_event_source_without_terminal_response() {
    let workspace = unique_temp_dir("hub-panel-failure").join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let (state, mut rx) = command_test_state(CapabilityProfile::Normal, workspace);
    let agent_id = state.config.read().await.agent_id.clone();
    let request_id = "req-panel-failure".to_string();
    let identity = hub::RunIdentity {
        run_id: "run-panel-failure".to_string(),
        request_id: request_id.clone(),
        command_hash: "command-hash-panel-failure".to_string(),
        agent_id: agent_id.clone(),
    };
    let expected_origin = agentic_gpt_protocol::EventOrigin {
        run_id: identity.run_id.clone(),
        request_id: identity.request_id.clone(),
        command_hash: identity.command_hash.clone(),
    };
    state
        .event_store
        .inject(
            &agentic_gpt_protocol::EventInjectRequest {
                message: "forces panel exposure update".to_string(),
                severity: Some(agentic_gpt_protocol::EventSeverity::Low),
                reference: "hub-panel-failure".to_string(),
            },
            state.config.read().await.events.low_ttl_seconds,
        )
        .unwrap();
    let database = state.private_state.root.join("events.sqlite3");
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_panel_exposure BEFORE UPDATE OF shown_count ON events
             BEGIN SELECT RAISE(ABORT, 'panel write failed'); END;",
        )
        .unwrap();

    let error = hub::handle_hub_command(
        state.clone(),
        HubCommand::Exec {
            request_id,
            payload: agentic_gpt_protocol::ProcessExecRequest {
                agent_id,
                group: None,
                program: "true".to_string(),
                args: Vec::new(),
                need_confirm: false,
                confirm_method: None,
                working_directory: None,
                wait_seconds: Some(5),
            },
        },
        Some(identity),
    )
    .await
    .expect_err("panel exposure SQL fault must prevent business response");
    assert!(error
        .to_string()
        .contains("event_store_panel_exposure_update_failed"));

    let message = rx.recv().await.unwrap();
    let AgentMessage::EventSources { origin, sources } = message else {
        panic!("expected identity-only event source handoff");
    };
    assert_eq!(origin, expected_origin);
    let process_source = sources
        .iter()
        .find(|source| source.kind == agentic_gpt_protocol::EventSourceKind::Process)
        .expect("real process execution source must be included");
    assert_eq!(
        state.event_store.remote_origin(process_source).unwrap(),
        Some(expected_origin)
    );
    assert!(state
        .event_store
        .pending_internal_sources()
        .unwrap()
        .contains(process_source));
    assert!(matches!(
        rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn room_mode_dispatches_bootstrap_manifest_and_read() {
    let workspace = unique_temp_dir("room-bootstrap-dispatch").join("workspace");
    let guides = workspace.join("bootstrap").join("guides");
    fs::create_dir_all(&guides).unwrap();
    fs::write(
            workspace.join("bootstrap").join("bootstrap.md"),
            "---\nid: room\nkind: entrypoint\nname: Room\ndescription: Route guides\nschemaVersion: 1\n---\nstart\n",
        )
        .unwrap();
    fs::write(
        guides.join("guide.md"),
        "---\nid: guide\nkind: guide\ntitle: Guide\nsummary: Use guide\n---\nbody\n",
    )
    .unwrap();
    let (state, mut rx) = command_test_state(CapabilityProfile::Room, workspace);

    hub::handle_hub_command(
        state.clone(),
        HubCommand::RoomBootstrap {
            request_id: "req-bootstrap".to_string(),
        },
        None,
    )
    .await
    .unwrap();
    let response = recv_response(&mut rx).await;
    assert_eq!(response["schemaVersion"], 1);
    assert_eq!(response["entrypoint"]["id"], "room");
    assert_eq!(response["guides"][0]["id"], "guide");

    hub::handle_hub_command(
        state,
        HubCommand::RoomBootstrapRead {
            request_id: "req-bootstrap-read".to_string(),
            payload: BootstrapReadRequest {
                id: "guide".to_string(),
            },
        },
        None,
    )
    .await
    .unwrap();
    let response = recv_response(&mut rx).await;
    assert_eq!(response["guide"]["id"], "guide");
    assert_eq!(
        response["resource"]["content"],
        "---\nid: guide\nkind: guide\ntitle: Guide\nsummary: Use guide\n---\nbody\n"
    );
}

#[tokio::test]
async fn normal_hub_rejects_current_room_commands_without_room_profile() {
    let workspace = unique_temp_dir("normal-room-command-rejected").join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let (state, mut rx) = command_test_state(CapabilityProfile::Normal, workspace);

    hub::handle_hub_command(
        state,
        HubCommand::RoomDiaryActive {
            request_id: "req-room".to_string(),
            payload: RoomDiaryActiveRequest::default(),
        },
        None,
    )
    .await
    .unwrap();
    let response = recv_response(&mut rx).await;
    assert_eq!(response["error"]["code"], "room_agent_required");
}

#[tokio::test]
async fn room_mode_dispatches_current_diary_command() {
    let workspace = unique_temp_dir("room-current-diary").join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let (state, mut rx) = command_test_state(CapabilityProfile::Room, workspace);

    hub::handle_hub_command(
        state,
        HubCommand::RoomDiaryActive {
            request_id: "req-room-diary".to_string(),
            payload: RoomDiaryActiveRequest::default(),
        },
        None,
    )
    .await
    .unwrap();
    let response = recv_response(&mut rx).await;
    assert_eq!(response["daily"]["path"], "Diary/Daily/current.md");
    assert_eq!(response["daily"]["available"], false);
    assert_eq!(response["daily"]["issue"], "missing");
}

#[test]
fn room_policy_overlay_differs_from_normal_policy() {
    let config = Config::default_config().unwrap();
    assert_eq!(
        policy::policy_decision_for_profile(&config, CapabilityProfile::Normal, "rm", &[], false),
        PolicyDecision::Confirm
    );
    assert_eq!(
        policy::policy_decision_for_profile(&config, CapabilityProfile::Room, "rm", &[], false),
        PolicyDecision::Allow
    );
}

#[test]
fn room_policy_keeps_high_risk_commands_restricted() {
    let config = Config::default_config().unwrap();
    for program in ["sudo", "scp", "mount", "systemctl", "service"] {
        assert_eq!(
            policy::policy_decision_for_profile(
                &config,
                CapabilityProfile::Room,
                program,
                &[],
                false
            ),
            PolicyDecision::Confirm
        );
    }
    assert_eq!(
        policy::policy_decision_for_profile(&config, CapabilityProfile::Room, "ssh", &[], false),
        PolicyDecision::Deny
    );
}

#[test]
fn rule_matches_program_and_args_prefix_structurally() {
    let rule = Rule {
        program: "python".to_string(),
        args_prefix: vec!["-c".to_string()],
    };
    assert!(rule.matches("python", &["-c".to_string(), "print(1)".to_string()]));
    assert!(!rule.matches("python3", &["-c".to_string()]));
    assert!(!rule.matches("python", &["script.py".to_string()]));
}

#[test]
fn safe_summary_includes_path_roots_and_policy_rules() {
    let root = unique_temp_dir("safe-summary");
    let workspace = root.join("workspace");
    let write_root = root.join("write");
    let read_only_root = root.join("readonly");
    let deny_root = root.join("deny");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&write_root).unwrap();
    fs::create_dir_all(&read_only_root).unwrap();
    fs::create_dir_all(&deny_root).unwrap();

    let mut config = Config::default_config().unwrap();
    config.workspace_root = workspace;
    config.path_policy = PathPolicyConfig {
        write_roots: vec![write_root.clone()],
        read_only_roots: vec![read_only_root.clone()],
        deny_roots: vec![deny_root.clone()],
    };
    config.policy.allow.push(Rule {
        program: "git".to_string(),
        args_prefix: vec!["status".to_string()],
    });
    config.policy.confirm.push(Rule {
        program: "bash".to_string(),
        args_prefix: vec!["-lc".to_string()],
    });
    config.policy.deny.push(Rule {
        program: "rm".to_string(),
        args_prefix: vec!["-rf".to_string()],
    });

    let summary = config.safe_summary();
    assert_eq!(summary.path_policy.write_root_count, 2);
    assert_eq!(summary.path_policy.read_only_root_count, 1);
    assert_eq!(summary.path_policy.deny_root_count, 1);
    assert!(summary
        .path_policy
        .write_roots
        .iter()
        .any(|root| root.path == "workspace" && root.source == "workspaceRoot"));
    assert!(summary
        .path_policy
        .write_roots
        .iter()
        .any(|root| root.path.ends_with("/write") && root.source == "configured"));
    assert!(summary
        .path_policy
        .read_only_roots
        .iter()
        .any(|root| root.path.ends_with("/readonly") && root.source == "configured"));
    assert!(summary
        .path_policy
        .deny_roots
        .iter()
        .any(|root| root.path.ends_with("/deny") && root.source == "configured"));

    assert_eq!(summary.policy_rule_counts.allow, 1);
    assert_eq!(summary.policy_rule_counts.confirm, 1);
    assert_eq!(summary.policy_rule_counts.deny, 1);
    assert!(summary
        .policy_rules
        .allow
        .iter()
        .any(|rule| { rule.program == "git" && rule.args_prefix == vec!["status".to_string()] }));
    assert!(summary
        .policy_rules
        .confirm
        .iter()
        .any(|rule| { rule.program == "bash" && rule.args_prefix == vec!["-lc".to_string()] }));
    assert!(summary
        .policy_rules
        .deny
        .iter()
        .any(|rule| { rule.program == "rm" && rule.args_prefix == vec!["-rf".to_string()] }));
    assert!(summary
        .policy_rules
        .builtins
        .confirm
        .iter()
        .any(|rule| rule.program == "bash" && rule.args_prefix.is_empty()));
    assert!(summary
        .policy_rules
        .builtins
        .confirm
        .iter()
        .any(|rule| rule.program == "python" && rule.args_prefix == vec!["-c".to_string()]));
    assert!(summary
        .policy_rules
        .builtins
        .deny
        .iter()
        .any(|rule| rule.program == "ssh" && rule.args_prefix.is_empty()));
}

#[test]
fn configured_allow_overrides_need_confirm() {
    let mut config = Config::default_config().unwrap();
    config.policy.allow.push(Rule {
        program: "git".to_string(),
        args_prefix: vec!["status".to_string()],
    });
    assert_eq!(
        policy::policy_decision_for_profile(
            &config,
            CapabilityProfile::Normal,
            "git",
            &["status".to_string()],
            true
        ),
        PolicyDecision::Allow
    );
}

#[test]
fn configured_allow_overrides_builtin_confirm() {
    let mut config = Config::default_config().unwrap();
    config.policy.allow.push(Rule {
        program: "curl".to_string(),
        args_prefix: vec!["--version".to_string()],
    });
    assert_eq!(
        policy::policy_decision_for_profile(
            &config,
            CapabilityProfile::Normal,
            "curl",
            &["--version".to_string()],
            false
        ),
        PolicyDecision::Allow
    );
}

#[test]
fn configured_allow_overrides_builtin_deny() {
    let mut config = Config::default_config().unwrap();
    config.policy.allow.push(Rule {
        program: "ssh".to_string(),
        args_prefix: vec!["-V".to_string()],
    });
    assert_eq!(
        policy::policy_decision_for_profile(
            &config,
            CapabilityProfile::Normal,
            "ssh",
            &["-V".to_string()],
            false
        ),
        PolicyDecision::Allow
    );
}

#[test]
fn configured_deny_wins_when_multiple_config_rules_match() {
    let mut config = Config::default_config().unwrap();
    config.policy.allow.push(Rule {
        program: "git".to_string(),
        args_prefix: vec![],
    });
    config.policy.deny.push(Rule {
        program: "git".to_string(),
        args_prefix: vec!["push".to_string()],
    });
    assert_eq!(
        policy::policy_decision_for_profile(
            &config,
            CapabilityProfile::Normal,
            "git",
            &["push".to_string()],
            false
        ),
        PolicyDecision::Deny
    );
}

#[test]
fn sudo_requires_credentials() {
    let config = Config::default_config().unwrap();
    assert_eq!(
        exec::preflight(
            &config,
            &config.workspace_root,
            "sudo",
            &["true".to_string()]
        )
        .unwrap_err(),
        "interactive_credential_required"
    );
}

#[test]
fn read_only_system_file_is_allowed() {
    let config = Config::default_config().unwrap();
    assert!(exec::preflight(
        &config,
        &config.workspace_root,
        "cat",
        &["/proc/meminfo".to_string()]
    )
    .is_ok());
    assert!(exec::preflight(&config, &config.workspace_root, "df", &["/".to_string()]).is_ok());
}

#[test]
fn path_policy_allows_write_root_and_blocks_readonly_write() {
    let root = unique_temp_dir("path-policy");
    let workspace = root.join("workspace");
    let downloads = root.join("Downloads");
    let cache = root.join(".cache");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&downloads).unwrap();
    fs::create_dir_all(&cache).unwrap();

    let mut config = Config::default_config().unwrap();
    config.workspace_root = workspace;
    config.path_policy = PathPolicyConfig {
        write_roots: vec![downloads.clone()],
        read_only_roots: vec![cache.clone()],
        deny_roots: Vec::new(),
    };

    assert!(exec::preflight(
        &config,
        &config.workspace_root,
        "touch",
        &[downloads.join("test-file").to_string_lossy().to_string()]
    )
    .is_ok());
    assert_eq!(
        exec::preflight(
            &config,
            &config.workspace_root,
            "touch",
            &[cache.join("test-file").to_string_lossy().to_string()]
        )
        .unwrap_err(),
        "path_readonly"
    );
    assert!(exec::preflight(
        &config,
        &config.workspace_root,
        "du",
        &[cache.to_string_lossy().to_string()]
    )
    .is_ok());
}

#[test]
fn deny_roots_override_read_and_write() {
    let root = unique_temp_dir("path-deny");
    let workspace = root.join("workspace");
    let downloads = root.join("Downloads");
    let secret = downloads.join("secret");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&secret).unwrap();

    let mut config = Config::default_config().unwrap();
    config.workspace_root = workspace;
    config.path_policy = PathPolicyConfig {
        write_roots: vec![downloads.clone()],
        read_only_roots: Vec::new(),
        deny_roots: vec![secret.clone()],
    };

    assert_eq!(
        exec::preflight(
            &config,
            &config.workspace_root,
            "cat",
            &[secret.join("token").to_string_lossy().to_string()]
        )
        .unwrap_err(),
        "path_denied"
    );
    assert_eq!(
        exec::preflight(
            &config,
            &config.workspace_root,
            "rm",
            &[secret.join("token").to_string_lossy().to_string()]
        )
        .unwrap_err(),
        "path_denied"
    );
}

#[test]
fn unknown_program_defaults_to_write_access() {
    let root = unique_temp_dir("path-unknown");
    let workspace = root.join("workspace");
    let cache = root.join(".cache");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&cache).unwrap();

    let mut config = Config::default_config().unwrap();
    config.workspace_root = workspace;
    config.path_policy = PathPolicyConfig {
        write_roots: Vec::new(),
        read_only_roots: vec![cache.clone()],
        deny_roots: Vec::new(),
    };

    assert_eq!(
        exec::preflight(
            &config,
            &config.workspace_root,
            "custom-tool",
            &[cache.join("file").to_string_lossy().to_string()]
        )
        .unwrap_err(),
        "path_readonly"
    );
}

#[test]
fn batch_confirmation_preview_supports_chinese() {
    let mut config = Config::default_config().unwrap();
    config.confirmation_language = "zh-CN".to_string();
    let element = PreparedBatchElement {
        index: 1,
        program: "python".to_string(),
        args: vec!["-c".to_string(), "print(1)".to_string()],
        working_directory: Some("/tmp".to_string()),
        resolved_working_directory: PathBuf::from("/tmp"),
        decision: PolicyDecision::Confirm,
    };
    let preview = confirmation::batch_confirmation_preview(
        &config,
        std::slice::from_ref(&element),
        std::slice::from_ref(&element),
    );

    assert!(preview.contains("该批次共有 1 条命令，其中 1 条需要确认"));
    assert!(preview.contains("工作目录：/tmp"));
    assert!(preview.contains("是否允许整个批次执行一次？"));
    assert!(!preview.contains("\\n"));
}

#[test]
fn working_directory_must_be_existing_writable_directory() {
    let root = unique_temp_dir("working-dir-policy");
    let workspace = root.join("workspace");
    let subdir = workspace.join("subdir");
    let cache = root.join("cache");
    let secret = workspace.join("secret");
    fs::create_dir_all(&subdir).unwrap();
    fs::create_dir_all(&cache).unwrap();
    fs::create_dir_all(&secret).unwrap();
    fs::write(workspace.join("file"), "not a directory").unwrap();

    let mut config = Config::default_config().unwrap();
    config.workspace_root = workspace.clone();
    config.path_policy = PathPolicyConfig {
        write_roots: Vec::new(),
        read_only_roots: vec![cache],
        deny_roots: vec![secret.clone()],
    };

    assert_eq!(
        exec::resolve_working_directory(&config, None).unwrap(),
        workspace.canonicalize().unwrap()
    );
    assert_eq!(
        exec::resolve_working_directory(&config, Some("subdir")).unwrap(),
        subdir.canonicalize().unwrap()
    );
    assert_eq!(
        exec::resolve_working_directory(&config, Some("file")).unwrap_err(),
        "working_directory_not_directory"
    );
    assert_eq!(
        exec::resolve_working_directory(&config, Some("missing")).unwrap_err(),
        "working_directory_not_found"
    );
    assert_eq!(
        exec::resolve_working_directory(&config, Some("secret")).unwrap_err(),
        "working_directory_denied"
    );
    assert_eq!(
        exec::resolve_working_directory(
            &config,
            Some(root.join("cache").to_string_lossy().as_ref())
        )
        .unwrap_err(),
        "working_directory_outside_allowed_roots"
    );
}

#[test]
fn relative_path_arguments_are_resolved_from_working_directory() {
    let root = unique_temp_dir("working-dir-relative");
    let workspace = root.join("workspace");
    let subdir = workspace.join("subdir");
    fs::create_dir_all(&subdir).unwrap();
    fs::write(subdir.join("target.txt"), "ok").unwrap();

    let mut config = Config::default_config().unwrap();
    config.workspace_root = workspace;
    config.path_policy = PathPolicyConfig {
        write_roots: Vec::new(),
        read_only_roots: Vec::new(),
        deny_roots: Vec::new(),
    };

    assert_eq!(
        exec::preflight(
            &config,
            &config.workspace_root,
            "cat",
            &["./target.txt".to_string()]
        )
        .unwrap_err(),
        "path_not_found"
    );
    assert!(exec::preflight(&config, &subdir, "cat", &["./target.txt".to_string()]).is_ok());
}

#[test]
fn symlink_to_denied_path_is_rejected() {
    let root = unique_temp_dir("path-symlink");
    let workspace = root.join("workspace");
    let secret = root.join("secret");
    let link = workspace.join("secret-link");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&secret).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&secret, &link).unwrap();

    let mut config = Config::default_config().unwrap();
    config.workspace_root = workspace;
    config.path_policy = PathPolicyConfig {
        write_roots: Vec::new(),
        read_only_roots: Vec::new(),
        deny_roots: vec![secret],
    };

    #[cfg(unix)]
    assert_eq!(
        exec::preflight(
            &config,
            &config.workspace_root,
            "cat",
            &[link.to_string_lossy().to_string()]
        )
        .unwrap_err(),
        "path_denied"
    );
}

#[test]
fn load_old_config_without_path_policy_adds_defaults() {
    let root = unique_temp_dir("old-config");
    let config_path = root.join("config.json");
    let config = Config::default_config().unwrap();
    let mut value = serde_json::to_value(config).unwrap();
    value.as_object_mut().unwrap().remove("pathPolicy");
    fs::write(&config_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    let loaded = Config::load(&config_path).unwrap();
    assert!(!loaded.path_policy.write_roots.is_empty());
    assert!(!loaded.path_policy.read_only_roots.is_empty());
    assert!(!loaded.path_policy.deny_roots.is_empty());
}

#[test]
fn load_partial_path_policy_uses_workspace_derived_defaults_for_missing_lists() {
    let root = unique_temp_dir("partial-config");
    let config_path = root.join("config.json");
    let mut value = serde_json::to_value(Config::default_config().unwrap()).unwrap();
    value["pathPolicy"] = serde_json::json!({
        "writeRoots": [root.join("write")]
    });
    fs::write(&config_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    let loaded = Config::load(&config_path).unwrap();
    assert_eq!(loaded.path_policy.write_roots.len(), 1);
    let defaults = crate::config::default_path_policy(&loaded.workspace_root);
    assert_eq!(loaded.path_policy.read_only_roots, defaults.read_only_roots);
    assert_eq!(loaded.path_policy.deny_roots, defaults.deny_roots);
}

#[test]
fn old_rule_ids_are_ignored_when_loading_config() {
    let root = unique_temp_dir("old-rule-id");
    let config_path = root.join("config.json");
    let mut value = serde_json::to_value(Config::default_config().unwrap()).unwrap();
    value["policy"]["allow"] = serde_json::json!([
        {
            "id": "legacy-id",
            "program": "bash",
            "argsPrefix": []
        }
    ]);
    fs::write(&config_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    let loaded = Config::load(&config_path).unwrap();
    assert_eq!(loaded.policy.allow.len(), 1);
    assert_eq!(loaded.policy.allow[0].program, "bash");
    assert!(serde_json::to_value(&loaded.policy.allow[0])
        .unwrap()
        .get("id")
        .is_none());
}

#[test]
fn remove_rule_matches_command_without_uuid() {
    let mut rules = vec![Rule {
        program: "bash".to_string(),
        args_prefix: Vec::new(),
    }];

    policy::remove_rule(&mut rules, "bash", &[]).unwrap();
    assert!(rules.is_empty());
}

#[test]
fn remove_rule_matches_command_and_args_prefix() {
    let mut rules = vec![
        Rule {
            program: "python".to_string(),
            args_prefix: vec!["-c".to_string()],
        },
        Rule {
            program: "python".to_string(),
            args_prefix: vec!["script.py".to_string()],
        },
    ];

    policy::remove_rule(&mut rules, "python", &["-c".to_string()]).unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].args_prefix, vec!["script.py".to_string()]);
}

#[test]
fn remove_rule_refuses_ambiguous_non_interactive_match() {
    let mut rules = vec![
        Rule {
            program: "bash".to_string(),
            args_prefix: Vec::new(),
        },
        Rule {
            program: "bash".to_string(),
            args_prefix: Vec::new(),
        },
    ];

    let error = policy::remove_rule_with_interactive(&mut rules, "bash", &[], false).unwrap_err();
    assert!(error.to_string().contains("multiple_matching_rules"));
    assert_eq!(rules.len(), 2);
}

#[test]
fn path_root_remove_matches_expanded_equivalent_path() {
    let root = unique_temp_dir("path-cli");
    let target = root.join("target");
    fs::create_dir_all(&target).unwrap();
    let mut policy = PathPolicyConfig::default();

    policy::mutate_path_roots(
        &mut policy,
        PathRootKind::Write,
        PathRootCommand::Add {
            path: target.clone(),
        },
    );
    assert_eq!(policy.write_roots.len(), 1);
    policy::mutate_path_roots(
        &mut policy,
        PathRootKind::Write,
        PathRootCommand::Remove {
            path: target.join("..").join("target"),
        },
    );
    assert!(policy.write_roots.is_empty());
}

#[test]
fn standalone_reload_replaces_the_frozen_live_subset() {
    let mut live = Config::default_config().unwrap();
    let original_agent_id = live.agent_id.clone();
    let original_workspace = live.workspace_root.clone();
    let original_path_policy = live.path_policy.clone();
    let mut candidate = live.clone();
    candidate.agent_id = "must-not-reload".to_string();
    candidate.workspace_root = PathBuf::from("/tmp/must-not-reload");
    candidate.policy.allow.push(Rule {
        program: "printf".to_string(),
        args_prefix: Vec::new(),
    });
    candidate.path_policy.write_roots = vec![PathBuf::from("/tmp/live")];
    candidate.limits.max_active_processes = config::MaxActiveProcesses::Explicit(9);
    candidate.limits.max_file_search_context_lines = 20;
    candidate.limits.process_response_bytes = agentic_gpt_protocol::MIN_PROCESS_RESPONSE_BYTES;
    live.mcp_servers.insert(
        "primary".to_string(),
        McpServerConfig {
            enabled: true,
            transport: "streamable-http".to_string(),
            url: Some("https://old.example/mcp".to_string()),
            auth: None,
        },
    );
    let in_flight = live.mcp_servers["primary"].clone();
    candidate.mcp_servers.insert(
        "primary".to_string(),
        McpServerConfig {
            enabled: false,
            transport: "streamable-http".to_string(),
            url: Some("https://new.example/mcp".to_string()),
            auth: None,
        },
    );

    let candidate_toolsets = candidate.toolsets.clone();
    let resolved = apply_live_config_subset(&mut live, candidate);

    assert_eq!(live.agent_id, original_agent_id);
    assert_eq!(live.workspace_root, original_workspace);
    assert!(live
        .policy
        .allow
        .iter()
        .any(|rule| rule.program == "printf"));
    assert_eq!(live.path_policy, original_path_policy);
    assert_eq!(resolved.resolved, 9);
    assert_eq!(
        live.limits.max_active_processes,
        config::MaxActiveProcesses::Explicit(9)
    );
    assert_eq!(live.limits.max_file_search_context_lines, 20);
    assert_eq!(
        live.limits.process_response_bytes,
        agentic_gpt_protocol::MIN_PROCESS_RESPONSE_BYTES
    );
    assert_eq!(live.toolsets, candidate_toolsets);
    assert_eq!(
        live.mcp_servers["primary"].url.as_deref(),
        Some("https://new.example/mcp")
    );
    assert!(!live.mcp_servers["primary"].enabled);
    assert_eq!(
        in_flight.url.as_deref(),
        Some("https://old.example/mcp"),
        "an already-cloned in-flight definition retains the old endpoint"
    );
}

#[tokio::test]
async fn wp2_reload_preserves_live_path_policy_when_workspace_changes() -> anyhow::Result<()> {
    let root = unique_temp_dir("standalone-live-path-policy-boundary");
    fs::create_dir_all(&root)?;
    let config_path = root.join("config.json");
    let live_workspace = root.join("workspace-live");
    let candidate_workspace = root.join("workspace-candidate");
    let live_write_root = live_workspace.join("write");
    let live_read_only_root = live_workspace.join("read-only");
    let live_deny_root = live_workspace.join("deny");
    let candidate_write_root = candidate_workspace.join("write");
    let candidate_read_only_root = candidate_workspace.join("read-only");
    let candidate_deny_root = candidate_workspace.join("deny");
    for path in [
        &live_write_root,
        &live_read_only_root,
        &live_deny_root,
        &candidate_write_root,
        &candidate_read_only_root,
        &candidate_deny_root,
    ] {
        fs::create_dir_all(path)?;
    }

    let (mut state, _rx) = command_test_state(CapabilityProfile::Normal, live_workspace.clone());
    state.config_path = config_path.clone();
    state.runtime = RuntimeModel::tunnel(CapabilityProfile::Normal, false);

    let mut initial = state.config.read().await.clone();
    initial.workspace_root = live_workspace.clone();
    initial.tunnel = Some(TunnelConfig {
        tunnel_id: "tunnel_test".to_string(),
        api_key: "env:AGENTIC_TUNNEL_API_KEY".to_string(),
        ..TunnelConfig::default()
    });
    initial.path_policy = PathPolicyConfig {
        write_roots: vec![live_write_root],
        read_only_roots: vec![live_read_only_root],
        deny_roots: vec![live_deny_root],
    };
    let live_path_policy = initial.path_policy.clone();
    *state.config.write().await = initial.clone();

    let mut candidate = initial;
    candidate.workspace_root = candidate_workspace;
    candidate.path_policy = PathPolicyConfig {
        write_roots: vec![candidate_write_root],
        read_only_roots: vec![candidate_read_only_root],
        deny_roots: vec![candidate_deny_root],
    };
    candidate.policy.allow.push(Rule {
        program: "printf".to_string(),
        args_prefix: Vec::new(),
    });
    fs::write(&config_path, serde_json::to_vec_pretty(&candidate)?)?;

    reload_live_config_once(&state).await?;

    let live = state.config.read().await.clone();
    assert_eq!(live.workspace_root, live_workspace);
    assert_eq!(live.path_policy, live_path_policy);
    assert!(live
        .policy
        .allow
        .iter()
        .any(|rule| rule.program == "printf"));

    let _ = fs::remove_dir_all(root);
    Ok(())
}
#[tokio::test]
async fn wp2_hub_live_reload_preserves_restart_fields() -> anyhow::Result<()> {
    let root = unique_temp_dir("hub-live-config-boundary");
    fs::create_dir_all(&root)?;
    let config_path = root.join("config.json");
    let live_workspace = root.join("workspace-live");
    let candidate_workspace = root.join("workspace-candidate");
    let live_write_root = live_workspace.join("write");
    let candidate_write_root = candidate_workspace.join("write");
    fs::create_dir_all(&live_write_root)?;
    fs::create_dir_all(&candidate_write_root)?;

    let (mut state, _rx) = command_test_state(CapabilityProfile::Normal, live_workspace.clone());
    state.config_path = config_path.clone();
    state.runtime = RuntimeModel::hub(CapabilityProfile::Normal);

    let mut initial = state.config.read().await.clone();
    initial.mode = RuntimeMode::Hub;
    initial.workspace_root = live_workspace.clone();
    initial.hub.agent_secret = "hub-secret".to_string();
    initial.path_policy.write_roots = vec![live_write_root];
    let live_hub_url = initial.hub.url.clone();
    let live_path_policy = initial.path_policy.clone();
    *state.config.write().await = initial.clone();

    let mut candidate = initial;
    candidate.hub.url = "http://127.0.0.1:18787".to_string();
    candidate.workspace_root = candidate_workspace;
    candidate.path_policy.write_roots = vec![candidate_write_root];
    candidate.policy.allow.push(Rule {
        program: "printf".to_string(),
        args_prefix: Vec::new(),
    });
    fs::write(&config_path, serde_json::to_vec_pretty(&candidate)?)?;

    reload_live_config_once(&state).await?;

    let live = state.config.read().await.clone();
    assert_eq!(live.mode, RuntimeMode::Hub);
    assert_eq!(live.hub.url, live_hub_url);
    assert_eq!(live.workspace_root, live_workspace);
    assert_eq!(live.path_policy, live_path_policy);
    assert!(live
        .policy
        .allow
        .iter()
        .any(|rule| rule.program == "printf"));

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[tokio::test]
async fn standalone_live_reload_applies_valid_mcp_map_and_rejects_invalid_candidate() {
    let root = unique_temp_dir("standalone-live-mcp-reload");
    fs::create_dir_all(&root).unwrap();
    let config_path = root.join("config.json");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let (mut state, _rx) = command_test_state(CapabilityProfile::Normal, workspace.clone());
    state.config_path = config_path.clone();
    state.runtime = RuntimeModel::tunnel(CapabilityProfile::Normal, false);

    let mut initial = state.config.read().await.clone();
    initial.workspace_root = workspace;
    initial.tunnel = Some(TunnelConfig {
        tunnel_id: "tunnel_test".to_string(),
        api_key: "env:AGENTIC_TUNNEL_API_KEY".to_string(),
        ..TunnelConfig::default()
    });
    initial.mcp_servers.insert(
        "primary".to_string(),
        McpServerConfig {
            enabled: true,
            transport: "streamable-http".to_string(),
            url: Some("https://old.example/mcp".to_string()),
            auth: None,
        },
    );
    *state.config.write().await = initial.clone();

    let mut valid = initial.clone();
    valid.mcp_servers.insert(
        "primary".to_string(),
        McpServerConfig {
            enabled: true,
            transport: "streamable-http".to_string(),
            url: Some("https://new.example/mcp".to_string()),
            auth: None,
        },
    );
    valid.mcp_servers.insert(
        "local".to_string(),
        McpServerConfig {
            enabled: false,
            transport: "stdio".to_string(),
            url: Some("node ./local-server.mjs".to_string()),
            auth: None,
        },
    );
    valid.limits.max_file_search_context_lines = 20;
    valid.limits.process_response_bytes = agentic_gpt_protocol::MAX_PROCESS_RESPONSE_BYTES;
    fs::write(&config_path, serde_json::to_vec_pretty(&valid).unwrap()).unwrap();
    reload_live_config_once(&state).await.unwrap();
    let live_after_valid = state.config.read().await.clone();
    assert_eq!(live_after_valid.mcp_servers, valid.mcp_servers);
    assert_eq!(live_after_valid.limits.max_file_search_context_lines, 20);
    assert_eq!(
        live_after_valid.limits.process_response_bytes,
        agentic_gpt_protocol::MAX_PROCESS_RESPONSE_BYTES
    );
    assert_eq!(live_after_valid.toolsets, valid.toolsets);

    let mut invalid = valid;
    invalid.mcp_servers.get_mut("primary").unwrap().transport = "sse".to_string();
    fs::write(&config_path, serde_json::to_vec_pretty(&invalid).unwrap()).unwrap();
    let error = reload_live_config_once(&state)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.starts_with("unsupported_mcp_transport"));
    assert_eq!(
        state.config.read().await.mcp_servers,
        live_after_valid.mcp_servers,
        "invalid disk changes must not partially replace the live map"
    );
    assert_eq!(
        state.config.read().await.limits.process_response_bytes,
        agentic_gpt_protocol::MAX_PROCESS_RESPONSE_BYTES
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn standalone_live_reload_bootstraps_room_repository_before_maintenance_submit(
) -> anyhow::Result<()> {
    let root = unique_temp_dir("standalone-live-room-reload");
    let config_path = root.join("config.json");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace)?;
    let (mut state, _rx) = command_test_state(CapabilityProfile::Normal, workspace.clone());
    state.config_path.clone_from(&config_path);
    state.runtime = RuntimeModel::tunnel(CapabilityProfile::Normal, false);
    let mut initial = state.config.read().await.clone();
    initial.workspace_root = workspace.clone();
    initial.tunnel = Some(TunnelConfig {
        tunnel_id: "tunnel_test".to_string(),
        api_key: "env:AGENTIC_TUNNEL_API_KEY".to_string(),
        ..TunnelConfig::default()
    });
    let live_room_root = workspace.join("room-live");
    let candidate_room_root = workspace.join("room-candidate");
    initial.room.repository_root = Some(live_room_root.clone());
    assert!(!initial.toolsets.is_enabled(config::ToolNamespace::Room));
    assert!(!room_repository::repository_root(&initial).exists());
    *state.config.write().await = initial.clone();

    let mut candidate = initial.clone();
    candidate.room.repository_root = Some(candidate_room_root.clone());
    candidate.toolsets.enable(config::ToolNamespace::Room);
    fs::write(&config_path, serde_json::to_vec_pretty(&candidate)?)?;

    reload_live_config_once(&state).await?;

    let live = state.config.read().await.clone();
    assert_eq!(live.room, initial.room);
    assert!(!candidate_room_root.exists());
    let room_root = room_repository::repository_root(&live);
    assert_eq!(room_root, live_room_root);
    assert!(room_root.join(".git").is_dir());
    for relative in room_repository::scaffold_paths() {
        assert!(
            room_root.join(relative).is_file(),
            "missing Room scaffold file: {relative}"
        );
    }

    let response = room_maintenance::submit(
        &state,
        agentic_gpt_protocol::RoomMaintenanceSubmitRequest {
            items: vec![agentic_gpt_protocol::RoomMaintenanceRequestItem {
                slot: agentic_gpt_protocol::RoomMaintenanceSlot::Notebook,
                payload: serde_json::json!({
                    "path": "Notebook/live-reload.md",
                    "title": "Live reload",
                    "body": "Bootstrapped",
                }),
            }],
            mode: Some(agentic_gpt_protocol::RoomMaintenanceExecutionMode::Local),
            wait_seconds: None,
        },
    )
    .await?;
    assert_eq!(
        response.state,
        agentic_gpt_protocol::RoomMaintenanceSubmissionState::Applied
    );
    assert!(response.local_applied);
    assert_eq!(
        fs::read_to_string(room_root.join("Notebook/live-reload.md"))?,
        "# Live reload\n\nBootstrapped\n"
    );

    let _ = fs::remove_dir_all(root);
    Ok(())
}

#[tokio::test]
async fn notification_delivery_rejects_unsupported_channel() {
    let response =
        notify::deliver_freedesktop_notification(agentic_gpt_protocol::UserNotifyDeliveryRequest {
            channel_key: "hub::ntfy".to_string(),
            title: "Hello".to_string(),
            body: "World".to_string(),
            actions: Vec::new(),
            priority: None,
        })
        .await;
    assert!(!response.delivered);
    assert_eq!(response.reason.as_deref(), Some("unsupported_channel"));
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    dir
}
