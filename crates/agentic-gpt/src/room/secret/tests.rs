//! Retained tiers ④/⑤: real subprocesses protect sync ownership across query
//! cancellation, strict disclosure confirmation, full-item preservation and
//! uncertain remote writes. Existing email/process tests cannot exercise this
//! private owner or its sensitive stdin/response boundary.
use super::*;
use futures_util::FutureExt;
use std::panic::AssertUnwindSafe;

const ITEM_ID: &str = "fabbb491-4e6d-40a3-86b8-6fb12a841919";
struct Fixture {
    root: PathBuf,
    state: AppState,
    owner: Arc<Owner>,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("agentic-secret-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut config = Config::default_config().unwrap();
        config.workspace_root = root.join("workspace");
        std::fs::create_dir_all(&config.workspace_root).unwrap();
        config.toolsets.enable(ToolNamespace::RoomSecret);
        config.policy.enabled = false;
        config.confirmation_provider.channels.clear();
        config.room.secret = RoomSecretConfig {
            server_url: "https://vault.example.invalid".into(),
            account: "test@example.invalid".into(),
            client_id: "env:UNUSED_TEST_CLIENT_ID".into(),
            client_secret: "env:UNUSED_TEST_CLIENT_SECRET".into(),
            master_password: "env:UNUSED_TEST_MASTER".into(),
            get_policy: SecretPolicy::Allow,
            upsert_policy: SecretPolicy::Allow,
            delete_policy: SecretPolicy::Allow,
            sync_wait_timeout_ms: 20,
        };
        let state = crate::startup::build_app_state_with_private_state(
            root.join("config.json"),
            config.clone(),
            crate::state::RuntimeModel::local(crate::state::CapabilityProfile::Room),
            false,
            PrivateStatePaths::for_test(root.join("state")),
        )
        .unwrap();
        let owner = state
            .secret
            .activate(&config.room.secret)
            .unwrap_or_else(|_| panic!("private test owner"));
        let binary = root.join("fake-bw.py");
        std::fs::write(&binary, FAKE_BW).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        *owner.cli.binary.write() = binary;
        std::fs::write(owner.cli.root.join("item.json"),serde_json::to_vec(&json!({"id":ITEM_ID,"type":1,"name":"Example","login":{"username":"Alice","password":"old-test-value","totp":"JBSWY3DPEHPK3PXP"},"notes":"retained notes","fields":[{"name":"retained","value":"custom"}],"deletedDate":null})).unwrap()).unwrap();
        Self { root, state, owner }
    }
    fn write(&self, name: &str, value: &str) {
        std::fs::write(self.owner.cli.root.join(name), value).unwrap();
    }
    async fn call(&self, operation: &str, args: Value) -> Value {
        dispatch(&self.state, operation, args).await
    }
    async fn wait_file(&self, name: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !self.owner.cli.root.join(name).exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
    async fn complete_sync(&self) {
        self.wait_file("sync-started").await;
        self.write("release-sync", "");
        let receiver = self
            .state
            .secret
            .job
            .lock()
            .await
            .as_ref()
            .unwrap()
            .result
            .clone();
        assert!(await_sync(receiver).await.is_ok());
    }
    async fn cleanup(self) {
        self.state.secret.shutdown().await;
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
const FAKE_BW: &str = r#"#!/usr/bin/python3
import os, sys, json, time, base64, pathlib
root=pathlib.Path(os.environ['BITWARDENCLI_APPDATA_DIR'])
assert pathlib.Path(os.environ['HOME']).is_relative_to(root)
assert 'NODE_OPTIONS' not in os.environ
args=sys.argv[1:]
assert all('new-test' not in arg for arg in args)
if args==['status']:
 print(json.dumps({'serverUrl':'https://vault.example.invalid','userEmail':'test@example.invalid','status':'unauthenticated' if (root/'unauthenticated').exists() else 'unlocked','lastSync':'2026-10-01T00:00:00Z'}))
elif args[:2]==['config','server']:print('Configured server.')
elif args==['login','--apikey']:
 assert os.environ.get('BW_CLIENTID') and os.environ.get('BW_CLIENTSECRET')
 print((root/'bootstrap-failure').read_text(),file=sys.stderr);sys.exit(1)
elif args==['sync']:
 with open(root/'sync-count','a') as f:f.write('1\n')
 (root/'sync-pid').write_text(str(os.getpid()))
 (root/'sync-started').touch()
 while not (root/'release-sync').exists():time.sleep(.005)
 if (root/'failure').exists():
  print((root/'failure').read_text()+' old-test-value',file=sys.stderr);sys.exit(1)
 print('Syncing complete.')
elif args==['list','items']:
 print('[]' if (root/'missing').exists() else '['+(root/'item.json').read_text()+']')
elif args[:2]==['get','item']:print((root/'item.json').read_text())
elif args[:2]==['get','totp']:print('123456')
elif args[:2] in (['edit','item'],['create','item']):
 item=json.loads(base64.b64decode(sys.stdin.buffer.read()))
 if args[0]=='edit':
  assert item['notes']=='retained notes'
  assert item['fields']==[{'name':'retained','value':'custom'}]
  assert item['login']['totp']=='JBSWY3DPEHPK3PXP'
 else:
  assert item['type']==1 and item['name']=='New site'
  assert item['login']['username']=='Bob'
  assert 'password' not in item['login']
  item['id']='fabbb491-4e6d-40a3-86b8-6fb12a841919'
  (root/'missing').unlink(missing_ok=True)
 with open(root/'write-count','a') as f:f.write('1\n')
 (root/'write-started').touch()
 if (root/'hold-write').exists():
  while not (root/'release-write').exists():time.sleep(.005)
 (root/'item.json').write_text(json.dumps(item))
 if (root/'write-unknown').exists():
  print('read ECONNRESET new-test-value',file=sys.stderr);sys.exit(1)
 print(json.dumps(item))
elif args[:2]==['delete','item']:(root/'missing').touch();print('Deleted item.')
elif args==['lock']:(root/'locked').touch()
else:sys.exit(2)
"#;

#[tokio::test]
async fn secret_disk_reload_updates_policy_wait_and_rejects_foreign_backend_identity() {
    // Tiers ④/⑤: protect disclosure/write authorization and waiting behavior
    // through disk reload and the native subprocess owner. A backend switch must
    // not disclose the old vault when the provider still reports its old identity.
    let fixture = Fixture::new();
    let result = AssertUnwindSafe(async {
        {
            let mut live = fixture.state.config.write().await;
            live.mode = crate::config::RuntimeMode::Local;
            live.profile = crate::config::WorkerProfile::Room;
        }
        let first = fixture.call("room.secret.list", json!({})).await;
        assert_eq!(first["stale"], true);

        let mut candidate = fixture.state.config.read().await.clone();
        candidate.room.secret.get_policy = SecretPolicy::Confirm;
        candidate.room.secret.upsert_policy = SecretPolicy::Confirm;
        candidate.room.secret.delete_policy = SecretPolicy::Confirm;
        candidate.room.secret.sync_wait_timeout_ms = 5000;
        std::fs::write(
            &fixture.state.config_path,
            serde_json::to_vec(&candidate).unwrap(),
        )
        .unwrap();
        let pending = crate::agent_info::collect(&fixture.state).await;
        assert_eq!(pending["config"]["liveSubsetMatchesDisk"], false);
        assert_eq!(pending["config"]["restartRequiredFields"], json!([]));
        crate::startup::reload_live_config_once(&fixture.state)
            .await
            .unwrap();
        let health = crate::agent_info::collect(&fixture.state).await;
        assert_eq!(health["config"]["liveSubsetMatchesDisk"], true);
        assert_eq!(health["config"]["restartRequiredFields"], json!([]));

        let waiting = fixture.call("room.secret.list", json!({}));
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut waiting)
                .await
                .is_err()
        );
        fixture.complete_sync().await;
        let listed = waiting.await;
        assert_eq!(listed["items"][0]["account"], "Alice");
        assert!(listed.get("stale").is_none());

        for (operation, args) in [
            (
                "room.secret.get",
                json!({"site":"Example","account":"Alice","type":"password"}),
            ),
            (
                "room.secret.upsert",
                json!({"site":"Example","account":"Alice","password":"new-test-value"}),
            ),
            (
                "room.secret.delete",
                json!({"site":"Example","account":"Alice"}),
            ),
        ] {
            let denied = fixture.call(operation, args).await;
            assert_eq!(denied["error"]["code"], "secret_confirmation_unavailable");
            assert!(!denied.to_string().contains("old-test-value"));
        }

        candidate.room.secret.server_url = "https://other-vault.example.invalid".into();
        candidate.room.secret.account = "other@example.invalid".into();
        candidate.room.secret.get_policy = SecretPolicy::Allow;
        std::fs::write(
            &fixture.state.config_path,
            serde_json::to_vec(&candidate).unwrap(),
        )
        .unwrap();
        crate::startup::reload_live_config_once(&fixture.state)
            .await
            .unwrap();
        let live = fixture.state.config.read().await.clone();
        let owner = fixture
            .state
            .secret
            .activate(&live.room.secret)
            .unwrap_or_else(|_| panic!("new backend must activate after disk reload"));
        *owner.cli.binary.write() = fixture.owner.cli.binary.read().clone();
        let disclosed = fixture
            .call(
                "room.secret.get",
                json!({"site":"Example","account":"Alice","type":"password"}),
            )
            .await;
        assert_eq!(disclosed["error"]["code"], "secret_authentication_failed");
        assert!(!disclosed.to_string().contains("old-test-value"));
        let health = crate::agent_info::collect(&fixture.state).await;
        assert_eq!(health["config"]["liveSubsetMatchesDisk"], true);
        assert_eq!(health["config"]["restartRequiredFields"], json!([]));
    })
    .catch_unwind()
    .await;
    fixture.cleanup().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
async fn secret_queries_share_sync_preserve_budget_and_fail_closed_confirmation() {
    let fixture = Fixture::new();
    let result = AssertUnwindSafe(async {
        let first = fixture.call("room.secret.list", json!({})).await;
        assert_eq!(first["stale"], true);
        assert_eq!(first["items"][0]["account"], "Alice");
        let triggered = fixture.state.secret.info()["syncLastTriggeredAt"].clone();
        fixture.write("missing", "");
        let missing = fixture
            .call(
                "room.secret.get",
                json!({"site":"example","account":"Alice","type":"password"}),
            )
            .await;
        assert_eq!(missing["error"]["code"], "secret_presence_unknown");
        assert_eq!(missing["lastSyncedAt"], "2026-10-01T00:00:00Z");
        assert_eq!(
            fixture.state.secret.info()["syncLastTriggeredAt"],
            triggered
        );
        assert_eq!(
            std::fs::read_to_string(fixture.owner.cli.root.join("sync-count"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        std::fs::remove_file(fixture.owner.cli.root.join("missing")).unwrap();
        fixture.state.config.write().await.room.secret.get_policy = SecretPolicy::Confirm;
        let denied = fixture
            .call(
                "room.secret.get",
                json!({"site":"Example","account":"Alice","type":"password"}),
            )
            .await;
        assert_eq!(denied["error"]["code"], "secret_confirmation_unavailable");
        assert!(!denied.to_string().contains("old-test-value"));
        assert_eq!(fixture.owner.usage.rank(ITEM_ID), (0.0, 0));
        fixture.complete_sync().await;
        assert_eq!(fixture.state.secret.info()["syncStatus"], "succeeded");
        let events = fixture
            .state
            .event_store
            .list(&agentic_gpt_protocol::EventListRequest::default())
            .unwrap();
        assert_eq!(events.items.len(), 1);
        let event = fixture
            .state
            .event_store
            .get(&events.items[0].event_id)
            .unwrap();
        assert_eq!(
            event.source.kind,
            agentic_gpt_protocol::EventSourceKind::Secret
        );
        assert_eq!(event.message, "密钥同步成功");
    })
    .catch_unwind()
    .await;
    fixture.cleanup().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
async fn secret_writes_merge_native_item_and_never_replay_uncertain_outcome() {
    let fixture = Fixture::new();
    fixture.write("release-sync", "");
    fixture
        .state
        .config
        .write()
        .await
        .room
        .secret
        .sync_wait_timeout_ms = 5000;
    let result = AssertUnwindSafe(async {
        let special = "new-test-value'\"\\\n雪";
        let updated = fixture
            .call(
                "room.secret.upsert",
                json!({"site":"example","account":"Alice","password":special}),
            )
            .await;
        assert_eq!(updated, json!({"updated":true}));
        let value = fixture
            .call(
                "room.secret.get",
                json!({"site":"EXAMPLE","account":"Alice","type":"password"}),
            )
            .await;
        assert_eq!(value["value"], special);
        fixture.write("write-unknown", "");
        let unknown = fixture
            .call(
                "room.secret.upsert",
                json!({"site":"Example","account":"Alice","password":"new-test-uncertain"}),
            )
            .await;
        assert_eq!(unknown["error"]["code"], "secret_write_outcome_unknown");
        assert!(!unknown.to_string().contains("new-test"));
        let persisted: Value = serde_json::from_slice(
            &std::fs::read(fixture.owner.cli.root.join("item.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(persisted["login"]["password"], "new-test-uncertain");
        let deleted = fixture
            .call(
                "room.secret.delete",
                json!({"site":"Example","account":"Alice"}),
            )
            .await;
        assert_eq!(deleted, json!({"deleted":true}));
        assert_eq!(fixture.owner.usage.rank(ITEM_ID), (0.0, 0));
        // Creation uses a different native command and needs no existing
        // password: omission must remain valid for a TOTP-only account.
        std::fs::remove_file(fixture.owner.cli.root.join("write-unknown")).unwrap();
        let created = fixture
            .call(
                "room.secret.upsert",
                json!({"site":"New site","account":"Bob","totp":"JBSWY3DPEHPK3PXP"}),
            )
            .await;
        assert_eq!(created, json!({"created":true}));
        let code = fixture
            .call(
                "room.secret.get",
                json!({"site":"NEW SITE","account":"Bob","type":"totp"}),
            )
            .await;
        assert_eq!(code["value"], "123456");
    })
    .catch_unwind()
    .await;
    fixture.cleanup().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
async fn secret_shutdown_reaps_sync_subprocess_after_query_returns() {
    let fixture = Fixture::new();
    let result = AssertUnwindSafe(async {
        let result = fixture.call("room.secret.list", json!({})).await;
        assert_eq!(result["stale"], true);
        fixture.wait_file("sync-pid").await;
        let pid: i32 = std::fs::read_to_string(fixture.owner.cli.root.join("sync-pid"))
            .unwrap()
            .parse()
            .unwrap();
        fixture.state.secret.shutdown().await;
        #[cfg(unix)]
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    })
    .catch_unwind()
    .await;
    fixture.cleanup().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

// Tier ⑤: shutdown alone does not prove that a stable service can release its
// account lease and reopen the same provider after configuration publication.
#[tokio::test]
async fn secret_reconfigure_reaps_old_sync_releases_owner_and_reopens_same_account() {
    let Fixture { root, state, owner } = Fixture::new();
    let provider_root = owner.cli.root.clone();
    let binary = owner.cli.binary.read().clone();
    // The fixture must not retain the very owner whose lease is under test.
    drop(owner);
    let result = AssertUnwindSafe(async {
        let response = dispatch(&state, "room.secret.list", json!({})).await;
        assert_eq!(response["stale"], true);
        tokio::time::timeout(Duration::from_secs(5), async {
            while !provider_root.join("sync-pid").exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let pid: i32 = std::fs::read_to_string(provider_root.join("sync-pid"))
            .unwrap()
            .parse()
            .unwrap();
        let guard = state.secret.prepare_reconfigure().await.unwrap();
        #[cfg(unix)]
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        let lease = crate::instance_lock::InstanceLock::acquire(
            &provider_root.join("vault"),
            ".owner.lock",
            "secret",
        )
        .expect("reconfiguration must release the old account lease");
        drop(lease);
        assert!(dispatch(&state, "room.secret.list", json!({})).await["error"].is_object());
        // In production the caller publishes its replacement Config here.
        drop(guard);
        let config = state.config.read().await.clone();
        let owner = state
            .secret
            .activate(&config.room.secret)
            .unwrap_or_else(|_| panic!("same account must activate after reconfiguration"));
        *owner.cli.binary.write() = binary;
        std::fs::write(provider_root.join("release-sync"), "").unwrap();
        let response = dispatch(&state, "room.secret.list", json!({})).await;
        assert!(response["items"].is_array(), "{response}");
        assert!(response.get("error").is_none(), "{response}");
    })
    .catch_unwind()
    .await;
    state.secret.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
async fn secret_only_known_transport_failure_allows_native_stale_reads() {
    let fixture = Fixture::new();
    fixture.write("release-sync", "");
    fixture
        .state
        .config
        .write()
        .await
        .room
        .secret
        .sync_wait_timeout_ms = 5000;
    let result = AssertUnwindSafe(async {
        fixture.write(
            "failure",
            "request failed: getaddrinfo ENOTFOUND private-host",
        );
        let offline = fixture.call("room.secret.list", json!({})).await;
        assert_eq!(offline["stale"], true);
        assert!(offline["items"].is_array());
        for (failure, code) in [
            ("401 Unauthorized", "secret_authentication_failed"),
            (
                "TLS certificate validation failed",
                "secret_provider_unavailable",
            ),
            ("unrecognized failure", "secret_provider_unavailable"),
        ] {
            fixture.write("failure", failure);
            let result = fixture.call("room.secret.list", json!({})).await;
            assert_eq!(result["error"]["code"], code);
            assert!(result.get("stale").is_none());
            assert!(!result.to_string().contains("old-test-value"));
        }
        // A real login subprocess is a separate owner path: compatibility and
        // TLS failures must not be rewritten as invalid credentials there.
        let reference = fixture.root.join("credential");
        std::fs::write(&reference, "synthetic-bootstrap-credential").unwrap();
        {
            let mut config = fixture.state.config.write().await;
            config.room.secret.client_id = format!("file:{}", reference.display());
            config.room.secret.client_secret = format!("file:{}", reference.display());
        }
        fixture.write("unauthenticated", "");
        for (failure, code) in [
            (
                "userDecryptionOptions is missing in response",
                "secret_provider_unavailable",
            ),
            (
                "TLS certificate validation failed",
                "secret_provider_unavailable",
            ),
            ("invalid client", "secret_authentication_failed"),
        ] {
            fixture.write("bootstrap-failure", failure);
            let value = fixture.call("room.secret.list", json!({})).await;
            assert_eq!(value["error"]["code"], code);
            assert!(value.get("stale").is_none());
        }
    })
    .catch_unwind()
    .await;
    fixture.cleanup().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

// The long-lived invariant is transaction serialization after response loss,
// not a particular CLI invocation count in an ordinary successful request.
#[tokio::test]
async fn secret_cancelled_caller_does_not_release_inflight_write_transaction() {
    let fixture = Fixture::new();
    fixture.write("release-sync", "");
    fixture.write("hold-write", "");
    let result = AssertUnwindSafe(async {
        let state = fixture.state.clone();
        let first = tokio::spawn(async move {
            dispatch(
                &state,
                "room.secret.upsert",
                json!({"site":"Example","account":"Alice","password":"new-test-first"}),
            )
            .await
        });
        fixture.wait_file("write-started").await;
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());

        let state = fixture.state.clone();
        let mut second = tokio::spawn(async move {
            dispatch(
                &state,
                "room.secret.upsert",
                json!({"site":"Example","account":"Alice","password":"new-test-second"}),
            )
            .await
        });
        assert!(tokio::time::timeout(Duration::from_millis(75), &mut second)
            .await
            .is_err());
        for counter in ["sync-count", "write-count"] {
            assert_eq!(
                std::fs::read_to_string(fixture.owner.cli.root.join(counter))
                    .unwrap()
                    .lines()
                    .count(),
                1
            );
        }
        fixture.write("release-write", "");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), second)
                .await
                .unwrap()
                .unwrap(),
            json!({"updated":true})
        );
        let item: Value = serde_json::from_slice(
            &std::fs::read(fixture.owner.cli.root.join("item.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(item["login"]["password"], "new-test-second");
        for counter in ["sync-count", "write-count"] {
            assert_eq!(
                std::fs::read_to_string(fixture.owner.cli.root.join(counter))
                    .unwrap()
                    .lines()
                    .count(),
                2
            );
        }
    })
    .catch_unwind()
    .await;
    fixture.cleanup().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
