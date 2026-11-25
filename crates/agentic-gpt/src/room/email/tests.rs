//! Retained integration coverage (tiers ④/⑤): these cases protect persisted
//! first-import boundaries, crash recovery and MIME/file ownership across reopen.
//! No existing subsystem test can detect email progress/outbox or body expiry
//! regressions, so each exercises the durable owner rather than helper constants.
use super::*;

struct Fixture {
    root: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "agentic-email-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }
    fn open(&self) -> Arc<EmailService> {
        let service =
            EmailService::open(&PrivateStatePaths::for_test(self.root.join("state"))).unwrap();
        service.activate("account").unwrap();
        service
    }
    fn state(&self, smtp_port: u16) -> AppState {
        let mut config = Config::default_config().unwrap();
        config.workspace_root = self.root.join("workspace");
        std::fs::create_dir_all(&config.workspace_root).unwrap();
        config.policy.enabled = false;
        config.confirmation_provider.channels.clear();
        config.toolsets.enable(ToolNamespace::RoomEmail);
        config.email.send_policy = EmailPolicy::Confirm;
        config.email.trash_policy = EmailPolicy::Confirm;
        let secret = self.root.join("password");
        std::fs::write(&secret, "isolated-test-password").unwrap();
        config.email.account = Some(EmailAccountConfig {
            imap_host: "127.0.0.1".to_owned(),
            imap_port: smtp_port,
            smtp_host: "127.0.0.1".to_owned(),
            smtp_port,
            imap_tls: crate::config::EmailTls::Implicit,
            smtp_tls: crate::config::EmailTls::Implicit,
            username: "sender@example.com".to_owned(),
            from: "sender@example.com".to_owned(),
            password: format!("file:{}", secret.display()),
        });
        crate::startup::build_app_state_with_private_state(
            self.root.join("config.json"),
            config,
            crate::state::RuntimeModel::local(crate::state::CapabilityProfile::Room),
            false,
            PrivateStatePaths::for_test(self.root.join("state")),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn received(uid: u32, raw: &str) -> network::RemoteMessage {
    network::RemoteMessage {
        uid,
        received_at: chrono::Utc::now(),
        unread: true,
        flagged: false,
        raw: raw.as_bytes().to_vec(),
        unavailable: None,
    }
}

async fn set_email_toolset(state: &AppState, enabled: bool) -> Result<()> {
    let _apply = state.config_runtime.apply_lock.lock().await;
    let mut desired = state.config.read().await.clone();
    if enabled {
        desired.toolsets.enable(ToolNamespace::RoomEmail);
    } else {
        desired.toolsets.disable(ToolNamespace::RoomEmail);
    }
    crate::config_runtime::apply_locked(state, desired).await
}

// Tier ⑤ coverage protects the actual owner and sync lifecycle across disable
// and re-enable. Existing tests cover persistence after process restart, but do
// not catch a disabled worker sleeping through a 24-hour interval after re-enable.
#[tokio::test]
async fn email_disable_releases_owner_and_reenable_resumes_sync_promptly() {
    let fixture = Fixture::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = fixture.state(port);
    {
        let mut config = state.config.write().await;
        config.toolsets.disable(ToolNamespace::RoomEmail);
        config.email.sync_interval_seconds = 86_400;
        assert!(!config.toolsets.uses_room_repository());
        assert!(!crate::room_repository::repository_root(&config).exists());
    }

    let listener_stopping = tokio_util::sync::CancellationToken::new();
    let (attempted_tx, mut attempted_rx) = tokio::sync::mpsc::unbounded_channel();
    let listener_task = tokio::spawn({
        let listener_stopping = listener_stopping.clone();
        async move {
            loop {
                tokio::select! {
                    _ = listener_stopping.cancelled() => return Ok::<_, anyhow::Error>(()),
                    accepted = listener.accept() => {
                        let (stream, _) = accepted?;
                        drop(stream);
                        if attempted_tx.send(()).is_err() {
                            return Ok(());
                        }
                    }
                }
            }
        }
    });
    let shutdown = tokio_util::sync::CancellationToken::new();
    let mut worker = tokio::spawn(run(state.clone(), shutdown.clone()));

    let outcome: Result<()> = async {
        set_email_toolset(&state, true).await?;
        tokio::time::timeout(std::time::Duration::from_secs(5), attempted_rx.recv())
            .await
            .map_err(|_| anyhow!("first_email_sync_attempt_timeout"))?
            .ok_or_else(|| anyhow!("email_listener_stopped_before_first_attempt"))?;

        set_email_toolset(&state, false).await?;
        let owner_lock = crate::instance_lock::InstanceLock::acquire(
            &state.email.database,
            ".owner.lock",
            "email-test",
        )
        .map_err(|_| anyhow!("email_owner_not_released_after_disable"))?;
        drop(owner_lock);

        set_email_toolset(&state, true).await?;
        tokio::time::timeout(std::time::Duration::from_secs(5), attempted_rx.recv())
            .await
            .map_err(|_| anyhow!("email_sync_did_not_resume_promptly"))?
            .ok_or_else(|| anyhow!("email_listener_stopped_before_second_attempt"))?;
        Ok(())
    }
    .await;

    shutdown.cancel();
    let worker_result = tokio::time::timeout(std::time::Duration::from_secs(5), &mut worker).await;
    if worker_result.is_err() {
        worker.abort();
        let _ = worker.await;
    }
    listener_stopping.cancel();
    let mut listener_task = listener_task;
    let listener_result =
        tokio::time::timeout(std::time::Duration::from_secs(5), &mut listener_task).await;
    if listener_result.is_err() {
        listener_task.abort();
        let _ = listener_task.await;
    }

    assert!(outcome.is_ok(), "lifecycle failed: {outcome:?}");
    assert!(
        matches!(worker_result, Ok(Ok(Ok(())))),
        "worker did not stop cleanly: {worker_result:?}"
    );
    assert!(
        matches!(listener_result, Ok(Ok(Ok(())))),
        "listener did not stop cleanly: {listener_result:?}"
    );
}

#[test]
fn email_import_boundary_and_notification_outbox_survive_reopen() {
    let fixture = Fixture::new();
    let service = fixture.open();
    let first = service.store.sync_record("account", 31, 5).unwrap();
    service.store.save_received("account",&first,&received(1,"From: sender@example.com\r\nSubject: Historical notice\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nA historical notice.")).unwrap();
    assert!(service.store.pending_events().unwrap().is_empty());
    drop(service);

    // Simulate restart halfway through the first import. The new server
    // boundary must not swallow mail that arrived after initial enablement.
    let service = fixture.open();
    let resumed = service.store.sync_record("account", 31, 10).unwrap();
    assert_eq!(resumed.last_uid, 1);
    assert_eq!(resumed.baseline, 5);
    service.store.save_received("account",&resumed,&received(6,"From: sender@example.com\r\nSubject: During downtime\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><head><title>ignored</title></head><body><h1>Account notice</h1><p>Available after restart.</p><script>secret_script()</script></body></html>")).unwrap();
    let events = service.store.pending_events().unwrap();
    assert_eq!(events.len(), 1);
    let id = events[0].0.clone();
    service
        .store
        .finish_sync(
            "account",
            &[
                network::RemoteFlags {
                    uid: 1,
                    unread: true,
                    flagged: false,
                },
                network::RemoteFlags {
                    uid: 6,
                    unread: false,
                    flagged: true,
                },
            ],
        )
        .unwrap();
    drop(service);

    let service = fixture.open();
    let progress = service.store.sync_record("account", 31, 100).unwrap();
    assert_eq!(progress.last_uid, 6);
    assert!(
        service.store.sync_status("account").unwrap()["initialSyncComplete"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(service.store.pending_events().unwrap()[0].0, id);
    let mail = service.store.get(&id).unwrap();
    assert!(!mail.unread);
    let content = service.store.read_image(&mail).unwrap();
    assert!(content.body.contains("Available after restart"));
    assert!(!content.body.contains("secret_script"));
    service.store.acknowledge_event(&id).unwrap();
    service.store.finish_sync("account", &[]).unwrap();
    assert!(service
        .store
        .list_page("account", None, None, None, None, 20)
        .unwrap()
        .is_empty());
    assert_eq!(
        service
            .store
            .search("account", "restart", "body")
            .unwrap()
            .0
            .len(),
        1
    );
    assert!(service
        .store
        .read_image(&service.store.get(&id).unwrap())
        .is_ok());
    // Changing server identity must not re-import history or change old IDs.
    assert!(service.store.sync_record("account", 32, 100).is_err());
    assert_eq!(
        service.store.sync_status("account").unwrap()["state"],
        "recoveryRequired"
    );
    assert!(service
        .store
        .read_image(&service.store.get(&id).unwrap())
        .is_ok());
    assert!(service.store.pending_events().unwrap().is_empty());
}

#[test]
fn email_pending_send_reopens_as_queryable_unknown_and_expiry_keeps_identity() {
    let fixture = Fixture::new();
    let service = fixture.open();
    let raw=b"From: sender@example.com\r\nTo: recipient@example.com\r\nSubject: Outbound record\r\nMessage-ID: <stable@example.com>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nRecorded before SMTP.";
    let id = service
        .store
        .create_send(
            "account",
            raw,
            "Recorded before SMTP.",
            "sender@example.com",
            &["recipient@example.com".to_owned()],
            &[],
            "Outbound record",
        )
        .unwrap();
    drop(service); // Crash before SMTP outcome persistence.
    let service = fixture.open();
    let mail = service.store.get(&id).unwrap();
    assert_eq!(mail.status, "unknown");
    assert_eq!(
        service
            .store
            .search("account", "Recorded", "body")
            .unwrap()
            .0[0]["ref"],
        id
    );
    assert_eq!(
        service.store.read_image(&mail).unwrap().body,
        "Recorded before SMTP."
    );
    assert!(service
        .store
        .list_page("account", None, None, None, None, 20)
        .unwrap()
        .is_empty());
    // Capacity expiry removes sensitive content but keeps stable references and
    // explicit status, rather than making an uncertain send disappear.
    service.store.cleanup(90, 1).unwrap();
    drop(service);
    let service = fixture.open();
    let mail = service.store.get(&id).unwrap();
    assert_eq!(mail.status, "unknown");
    assert_eq!(
        service.store.read_image(&mail).err().unwrap().to_string(),
        "email_content_expired"
    );
    assert_eq!(
        service
            .store
            .search("account", "Outbound", "subject")
            .unwrap()
            .0[0]["ref"],
        id
    );
    assert!(!fixture
        .root
        .join("state/email")
        .join(format!("{id}.eml"))
        .exists());
}

#[test]
fn email_saved_mime_keeps_plain_body_and_attachment_bytes_after_reopen() {
    let fixture = Fixture::new();
    let service = fixture.open();
    let progress = service.store.sync_record("account", 7, 0).unwrap();
    let raw=concat!(
        "From: sender@example.com\r\nTo: reader@example.com\r\nSubject: Charset and attachment\r\n",
        "MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=mixed\r\n\r\n",
        "--mixed\r\nContent-Type: multipart/alternative; boundary=alternative\r\n\r\n",
        "--alternative\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n \t\r\n",
        "--alternative\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n",
        "Plain body =E4=BD=A0=E5=A5=BD\r\n",
        "--alternative\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>HTML alternate</p>\r\n--alternative--\r\n",
        "--mixed\r\nContent-Type: application/octet-stream; name=report.bin\r\n",
        "Content-Disposition: attachment; filename=\"../../report.bin\"\r\nContent-Transfer-Encoding: base64\r\n\r\nAAEC/w==\r\n--mixed--\r\n"
    );
    service
        .store
        .save_received("account", &progress, &received(1, raw))
        .unwrap();
    let id = service.store.pending_events().unwrap()[0].0.clone();
    drop(service);
    // Tier ⑤: a real schema-1 archive must migrate without an eager EML scan,
    // backfill attachment metadata once, and retain its normalized body. This
    // also catches empty MIME alternatives hiding a later usable plain body.
    let conn = rusqlite::Connection::open(fixture.root.join("state/email.sqlite3")).unwrap();
    conn.execute_batch(
        "UPDATE mail SET body_bytes=length(CAST(body AS BLOB));
         UPDATE stats SET value=(SELECT SUM(raw_bytes+body_bytes) FROM mail) WHERE key='content_bytes';
         ALTER TABLE mail DROP COLUMN attachments; PRAGMA user_version=1;",
    ).unwrap();
    drop(conn);
    let raw_path = fixture.root.join("state/email").join(format!("{id}.eml"));
    let moved = raw_path.with_extension("held");
    std::fs::rename(&raw_path, &moved).unwrap();
    let service = fixture.open();
    std::fs::rename(&moved, &raw_path).unwrap();
    let content = service
        .store
        .read_image(&service.store.get(&id).unwrap())
        .unwrap();
    assert_eq!(content.body, "Plain body 你好");
    assert_eq!(content.format, "plain");
    assert_eq!(content.attachments.len(), 1);
    assert_eq!(content.attachments[0].name, "report.bin");
    let mail = service.store.get(&id).unwrap();
    let attachments = service.store.attachment_metadata(&mail).unwrap();
    assert_eq!(attachments[0].size, 4);
    assert_eq!(
        service
            .store
            .read_attachment(&mail, "attachment_1")
            .unwrap()
            .bytes,
        vec![0, 1, 2, 255]
    );
    drop(service);
    std::fs::rename(&raw_path, &moved).unwrap();
    let service = fixture.open();
    let attachments = service
        .store
        .attachment_metadata(&service.store.get(&id).unwrap())
        .unwrap();
    assert_eq!(attachments[0].name, "report.bin");
    assert_eq!(attachments[0].size, 4);
    std::fs::rename(moved, raw_path).unwrap();
    assert_eq!(
        service.store.search("account", "你好", "body").unwrap().0[0]["ref"],
        id
    );
}

// Tier ⑤: exercise the public email dispatch, the actual confirmation provider
// decision and a listening TCP socket. This protects against accidentally using
// global policy.enabled=false as authorization for SMTP/IMAP side effects.
#[tokio::test]
async fn email_unavailable_confirmation_never_connects_smtp_or_trash_imap() {
    let fixture = Fixture::new();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let state = fixture.state(listener.local_addr().unwrap().port());
    let response=dispatch(&state,"room.email.send",json!({"to":["recipient@example.com"],"subject":"Needs actual approval","text":"Do not connect without approval."})).await;
    assert_eq!(
        response["error"]["code"],
        "email_confirmation_provider_unavailable"
    );
    let id = response["id"]
        .as_str()
        .expect("a rejected prepared send remains identifiable");
    assert_eq!(state.email.store.get(id).unwrap().status, "denied");
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );

    let account = state.config.read().await.email.account.clone().unwrap();
    let key = account_key(&account);
    let record = state.email.store.sync_record(&key, 1, 0).unwrap();
    state
        .email
        .store
        .save_received(
            &key,
            &record,
            &received(
                1,
                "From: sender@example.com\r\nSubject: Trash requires approval\r\n\r\nBody",
            ),
        )
        .unwrap();
    let mail_id = state.email.store.pending_events().unwrap()[0].0.clone();
    let response = dispatch(
        &state,
        "room.email.operate",
        json!({"id":mail_id,"action":"trash"}),
    )
    .await;
    assert_eq!(
        response["error"]["code"],
        "email_confirmation_provider_unavailable"
    );
    assert!(state.email.store.get(&mail_id).unwrap().inbox);
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

// Tier ⑤: a real full EventStore must leave durable mail progress and the
// producer outbox intact. Re-delivering after the receipt/ACK crash boundary
// must not create a second event, including after event history was removed.
#[tokio::test]
async fn email_event_capacity_and_ack_replay_do_not_block_mail_or_duplicate_reminders() {
    let fixture = Fixture::new();
    let state = fixture.state(1);
    let account = state.config.read().await.email.account.clone().unwrap();
    let key = account_key(&account);
    let record = state.email.store.sync_record(&key, 8, 0).unwrap();
    let mut flags = Vec::new();
    for uid in 1..=2 {
        state.email.store.save_received(&key,&record,&received(uid,&format!("From: sender@example.com\r\nSubject: Notice {uid}\r\n\r\nDurable body {uid}."))).unwrap();
        flags.push(network::RemoteFlags {
            uid,
            unread: true,
            flagged: false,
        });
    }
    state.email.store.finish_sync(&key, &flags).unwrap();
    let database = state.private_state.root.join("events.sqlite3");
    let conn = rusqlite::Connection::open(&database).unwrap();
    conn.execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<8192)
        INSERT INTO events(event_id,message,severity,created_at,status,source_kind,source_ref,shown_count)
        SELECT 'filler-'||x,'queued','medium',?,'pending','external','filler-'||x,0 FROM n",[chrono::Utc::now().to_rfc3339()]).unwrap();
    deliver_events(&state).await.unwrap();
    assert_eq!(state.email.store.pending_events().unwrap().len(), 2);
    assert_eq!(
        state.email.store.sync_record(&key, 8, 99).unwrap().last_uid,
        2
    );
    assert_eq!(
        state
            .email
            .store
            .list_page(&key, None, None, None, None, 20)
            .unwrap()
            .len(),
        2
    );
    conn.execute("DELETE FROM events", []).unwrap();

    // First event receipt commits, but producer ACK is lost before restart.
    let (first_id, message) = state.email.store.pending_events().unwrap()[0].clone();
    state
        .event_store
        .inject_external_once(
            &format!("email:{first_id}"),
            agentic_gpt_protocol::EventSourceKind::Email,
            &EventInjectRequest {
                message,
                severity: Some(EventSeverity::Medium),
                reference: first_id.clone(),
            },
            86400,
        )
        .unwrap();
    let service = fixture.open();
    assert_eq!(service.store.pending_events().unwrap().len(), 2);
    drop(service);
    deliver_events(&state).await.unwrap();
    assert!(state.email.store.pending_events().unwrap().is_empty());
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);
    let email_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE source_kind='email'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(email_count, 2);
    // Keep the producer-side ACK missing again after history cleanup. The
    // permanent delivery receipt must still prevent an old reminder replay.
    conn.execute("DELETE FROM events WHERE source_ref=?", [&first_id])
        .unwrap();
    let email_conn =
        rusqlite::Connection::open(state.private_state.root.join("email.sqlite3")).unwrap();
    email_conn
        .execute(
            "INSERT INTO outbox(id,message) VALUES(?,'old replay')",
            [&first_id],
        )
        .unwrap();
    deliver_events(&state).await.unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    assert!(state.email.store.pending_events().unwrap().is_empty());
}

#[test]
fn email_unreadable_mail_progress_and_explicit_recovery_keep_old_references() {
    let fixture = Fixture::new();
    let service = fixture.open();
    let record = service.store.sync_record("account", 1, 0).unwrap();
    let mut oversized = received(
        1,
        "From: sender@example.com\r\nSubject: Oversized delivery\r\n\r\n",
    );
    oversized.unavailable = Some("tooLarge");
    service
        .store
        .save_received("account", &record, &oversized)
        .unwrap();
    service
        .store
        .save_received(
            "account",
            &record,
            &received(
                2,
                "From: sender@example.com\r\nSubject: After oversized mail\r\n\r\nStill delivered.",
            ),
        )
        .unwrap();
    let old = service
        .store
        .list_page("account", None, None, None, None, 20)
        .unwrap();
    let too_large = old.iter().find(|mail| mail.uid == Some(1)).unwrap();
    assert_eq!(
        service
            .store
            .read_image(too_large)
            .err()
            .unwrap()
            .to_string(),
        "email_message_too_large"
    );
    let readable = old.iter().find(|mail| mail.uid == Some(2)).unwrap();
    let old_id = readable.id.clone();
    assert_eq!(
        service
            .store
            .sync_record("account", 1, 10)
            .unwrap()
            .last_uid,
        2
    );
    assert!(service.store.sync_record("account", 2, 5).is_err());
    let recovery = service
        .store
        .recover_account("account", 2, 5, false)
        .unwrap();
    assert_eq!(recovery["preservedHistoricalCopies"], 2);
    assert!(service.store.get(&old_id).unwrap().uid.is_none());
    assert_eq!(
        service
            .store
            .read_image(&service.store.get(&old_id).unwrap())
            .unwrap()
            .body,
        "Still delivered."
    );
    let after = service.store.sync_record("account", 2, 100).unwrap();
    assert_eq!(after.baseline, 5);
    service
        .store
        .save_received(
            "account",
            &after,
            &received(
                1,
                "Subject: Reimported historical copy\r\n\r\nHistorical copy.",
            ),
        )
        .unwrap();
    assert_eq!(service.store.pending_events().unwrap().len(), 2); // prior notices preserved, no historical replay
    service
        .store
        .save_received(
            "account",
            &after,
            &received(6, "Subject: New mail after recovery\r\n\r\nA new delivery."),
        )
        .unwrap();
    assert_eq!(service.store.pending_events().unwrap().len(), 3);
    drop(service);
    let service = fixture.open();
    assert_eq!(
        service
            .store
            .sync_record("account", 2, 999)
            .unwrap()
            .last_uid,
        6
    );
    assert!(service
        .store
        .read_image(&service.store.get(&old_id).unwrap())
        .is_ok());
}

// Tier ⑤: stored IDs from a previous account must not bypass the currently
// configured account's ownership boundary through any of the six tool paths.
#[tokio::test]
async fn email_current_account_cannot_read_or_operate_foreign_mail_ids() {
    let fixture = Fixture::new();
    let state = fixture.state(1);
    let foreign = state
        .email
        .store
        .sync_record("different-account", 9, 0)
        .unwrap();
    state.email.store.save_received("different-account",&foreign,&received(1,"From: private@example.com\r\nSubject: Foreign private notice\r\n\r\nForeign private content.")).unwrap();
    let foreign_id = state.email.store.pending_events().unwrap()[0].0.clone();
    for (operation, args) in [
        ("room.email.read", json!({"id":foreign_id})),
        (
            "room.email.attachment",
            json!({"id":foreign_id,"attachmentId":"attachment_1"}),
        ),
        (
            "room.email.operate",
            json!({"id":foreign_id,"action":"markRead"}),
        ),
        (
            "room.email.send",
            json!({"to":["reader@example.com"],"subject":"Reply","text":"No foreign thread access","replyToId":foreign_id}),
        ),
    ] {
        let response = dispatch(&state, operation, args).await;
        assert_eq!(response["error"]["code"], "email_not_found", "{operation}");
    }
    let response = dispatch(&state, "room.email.search", json!({"query":"Foreign"})).await;
    assert!(response["items"].as_array().unwrap().is_empty());
    let response = dispatch(&state, "room.email.list", json!({})).await;
    assert!(response["items"].as_array().unwrap().is_empty());
    assert!(state.email.store.get(&foreign_id).unwrap().unread);
}

// A bounded one-session TCP owner for the actual TLS protocol tests below.
// Every path joins its thread; private keys exist only in process memory.
struct ProtocolServer {
    port: u16,
    accepted: Arc<std::sync::atomic::AtomicUsize>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    task: Option<std::thread::JoinHandle<Result<Vec<String>>>>,
}
impl ProtocolServer {
    fn start(
        handler: impl FnOnce(std::net::TcpStream) -> Result<Vec<String>> + Send + 'static,
    ) -> Self {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(AtomicUsize::new(0));
        let count = accepted.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let task = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error.into()),
                }
                if stopping.load(Ordering::Acquire) || std::time::Instant::now() >= deadline {
                    return Err(anyhow!("email_test_server_accept_timeout"));
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            count.fetch_add(1, Ordering::AcqRel);
            stream.set_read_timeout(Some(std::time::Duration::from_secs(3)))?;
            stream.set_write_timeout(Some(std::time::Duration::from_secs(3)))?;
            let transcript = handler(stream)?;
            // Observe and immediately close any unexpected retry connections
            // until the client finishes. This also bounds a buggy retry loop.
            loop {
                while let Ok((stream, _)) = listener.accept() {
                    count.fetch_add(1, Ordering::AcqRel);
                    drop(stream);
                }
                if stopping.load(Ordering::Acquire) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Ok(transcript)
        });
        Self {
            port,
            accepted,
            stop,
            task: Some(task),
        }
    }
    fn finish(mut self) -> Vec<String> {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        self.task
            .take()
            .unwrap()
            .join()
            .expect("protocol server thread")
            .expect("protocol session")
    }
}
impl Drop for ProtocolServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

fn local_tls_identity() -> (
    rustls::pki_types::CertificateDer<'static>,
    Arc<rustls::ServerConfig>,
) {
    let identity = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let certificate = identity.cert.der().clone();
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(identity.key_pair.serialize_der());
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![certificate.clone()], key.into())
    .unwrap();
    (certificate, Arc::new(config))
}

fn wire_read<T: std::io::Read>(reader: &mut std::io::BufReader<T>) -> Result<String> {
    use std::io::BufRead;
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 || line.len() > 1024 * 1024 {
        return Err(anyhow!("email_test_unexpected_eof"));
    }
    Ok(line)
}
fn wire_send<T: std::io::Read + std::io::Write>(
    reader: &mut std::io::BufReader<T>,
    response: &str,
) -> Result<()> {
    reader.get_mut().write_all(response.as_bytes())?;
    reader.get_mut().flush()?;
    Ok(())
}
fn imap_reply<T: std::io::Read + std::io::Write>(
    reader: &mut std::io::BufReader<T>,
    request: &str,
    untagged: &str,
) -> Result<()> {
    let tag = request
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("email_test_imap_tag_missing"))?;
    wire_send(reader, &format!("{untagged}{tag} OK completed\r\n"))
}

#[tokio::test]
async fn email_real_tls_imap_peek_imports_without_marking_read() {
    for mode in [
        crate::config::EmailTls::Starttls,
        crate::config::EmailTls::Implicit,
    ] {
        let fixture = Fixture::new();
        let (certificate, tls) = local_tls_identity();
        let server = ProtocolServer::start(move |stream| {
            let mut transcript = Vec::new();
            let stream = if mode == crate::config::EmailTls::Starttls {
                let mut plain = std::io::BufReader::new(stream);
                wire_send(&mut plain, "* OK local IMAP fixture\r\n")?;
                let request = wire_read(&mut plain)?;
                assert!(request.to_ascii_uppercase().contains(" STARTTLS"));
                imap_reply(&mut plain, &request, "")?;
                transcript.push("STARTTLS before authentication".to_owned());
                plain.into_inner()
            } else {
                stream
            };
            let connection = rustls::ServerConnection::new(tls)?;
            let mut reader = std::io::BufReader::new(rustls::StreamOwned::new(connection, stream));
            if mode == crate::config::EmailTls::Implicit {
                wire_send(&mut reader, "* OK local IMAP fixture\r\n")?;
            }
            let login = wire_read(&mut reader)?;
            assert!(login.to_ascii_uppercase().contains(" LOGIN "));
            imap_reply(&mut reader, &login, "")?;
            transcript.push("TLS LOGIN".to_owned());
            let select = wire_read(&mut reader)?;
            assert!(select.to_ascii_uppercase().contains(" SELECT "));
            imap_reply(&mut reader,&select,"* FLAGS (\\Seen \\Flagged)\r\n* 1 EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY 19] Validity\r\n* OK [UIDNEXT 2] Next UID\r\n")?;
            let search = wire_read(&mut reader)?;
            assert!(search.to_ascii_uppercase().contains(" UID SEARCH ALL"));
            imap_reply(&mut reader, &search, "* SEARCH 1\r\n")?;
            let raw="From: sender@example.com\r\nTo: \"Reader, Example\" <reader@example.com>\r\nSubject: Real TLS delivery\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nVerified delivery over TLS.";
            let size = wire_read(&mut reader)?;
            assert!(size.to_ascii_uppercase().contains("RFC822.SIZE"));
            imap_reply(
                &mut reader,
                &size,
                &format!("* 1 FETCH (UID 1 RFC822.SIZE {})\r\n", raw.len()),
            )?;
            let fetch = wire_read(&mut reader)?;
            assert!(fetch.to_ascii_uppercase().contains("BODY.PEEK[]"));
            imap_reply(&mut reader,&fetch,&format!("* 1 FETCH (UID 1 FLAGS () INTERNALDATE \"24-Oct-2026 12:00:00 +0000\" BODY[] {{{}}}\r\n{raw})\r\n",raw.len()))?;
            transcript.push("UID BODY.PEEK[]".to_owned());
            let flags = wire_read(&mut reader)?;
            assert!(flags
                .to_ascii_uppercase()
                .contains("UID FETCH 1 (UID FLAGS)"));
            imap_reply(&mut reader, &flags, "* 1 FETCH (UID 1 FLAGS ())\r\n")?;
            Ok(transcript)
        });
        let state = fixture.state(server.port);
        let mut account = state.config.read().await.email.account.clone().unwrap();
        account.imap_host = "localhost".to_owned();
        account.imap_tls = mode;
        let key = account_key(&account);
        let service = state.email.clone();
        let id = blocking(move || {
            let mut inbox = network::inbox_with_test_root(&account, certificate)?;
            let record = service
                .store
                .sync_record(&key, inbox.validity, inbox.baseline)?;
            let message = inbox.message(1)?.expect("server fixture message");
            service.store.save_received(&key, &record, &message)?;
            let flags = inbox.flags(&[1])?;
            service.store.finish_sync(&key, &flags)?;
            Ok(
                service.store.list_page(&key, None, None, None, None, 20)?[0]
                    .id
                    .clone(),
            )
        })
        .await
        .unwrap();
        let count = server.accepted.clone();
        let transcript = server.finish();
        assert_eq!(count.load(std::sync::atomic::Ordering::Acquire), 1);
        assert!(transcript.iter().any(|line| line == "UID BODY.PEEK[]"));
        let mail = state.email.store.get(&id).unwrap();
        assert!(mail.unread);
        let content = state.email.store.read_image(&mail).unwrap();
        assert_eq!(content.body, "Verified delivery over TLS.");
        assert_eq!(
            mail_metadata(
                &mail,
                &state.email.store.attachment_metadata(&mail).unwrap()
            )["to"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}

fn smtp_session(
    stream: std::net::TcpStream,
    tls: Arc<rustls::ServerConfig>,
    starttls: bool,
    disconnect_after_data: bool,
) -> Result<Vec<String>> {
    let mut transcript = Vec::new();
    let stream = if starttls {
        let mut plain = std::io::BufReader::new(stream);
        wire_send(&mut plain, "220 localhost ESMTP fixture\r\n")?;
        assert!(wire_read(&mut plain)?.starts_with("EHLO "));
        wire_send(&mut plain, "250-localhost\r\n250 STARTTLS\r\n")?;
        assert_eq!(wire_read(&mut plain)?.trim(), "STARTTLS");
        wire_send(&mut plain, "220 Ready for TLS\r\n")?;
        transcript.push("STARTTLS before AUTH".to_owned());
        plain.into_inner()
    } else {
        stream
    };
    let connection = rustls::ServerConnection::new(tls)?;
    let mut reader = std::io::BufReader::new(rustls::StreamOwned::new(connection, stream));
    if !starttls {
        wire_send(&mut reader, "220 localhost ESMTP fixture\r\n")?;
    }
    assert!(wire_read(&mut reader)?.starts_with("EHLO "));
    wire_send(
        &mut reader,
        "250-localhost\r\n250-AUTH PLAIN\r\n250 SIZE 10000000\r\n",
    )?;
    let auth = wire_read(&mut reader)?;
    assert!(auth.starts_with("AUTH PLAIN"));
    if auth.trim() == "AUTH PLAIN" {
        wire_send(&mut reader, "334 \r\n")?;
        let _ = wire_read(&mut reader)?;
    }
    wire_send(&mut reader, "235 Authentication successful\r\n")?;
    assert!(wire_read(&mut reader)?.starts_with("MAIL FROM:"));
    wire_send(&mut reader, "250 Sender accepted\r\n")?;
    assert!(wire_read(&mut reader)?.starts_with("RCPT TO:"));
    wire_send(&mut reader, "250 Recipient accepted\r\n")?;
    assert_eq!(wire_read(&mut reader)?.trim(), "DATA");
    wire_send(&mut reader, "354 End with a dot\r\n")?;
    let mut body = String::new();
    loop {
        let line = wire_read(&mut reader)?;
        if line == ".\r\n" {
            break;
        }
        body.push_str(&line);
    }
    assert!(body.contains("Approved body preview"));
    transcript.push("DATA accepted once".to_owned());
    if !disconnect_after_data {
        wire_send(&mut reader, "250 Accepted for delivery\r\n")?;
    }
    Ok(transcript)
}

#[tokio::test]
async fn email_real_tls_smtp_follows_approval_and_disconnect_stays_unknown_without_retry() {
    for (mode, disconnect) in [
        (crate::config::EmailTls::Starttls, false),
        (crate::config::EmailTls::Implicit, true),
    ] {
        let fixture = Fixture::new();
        let (certificate, tls) = local_tls_identity();
        let server = ProtocolServer::start(move |stream| {
            smtp_session(
                stream,
                tls,
                mode == crate::config::EmailTls::Starttls,
                disconnect,
            )
        });
        let state = fixture.state(server.port);
        {
            let mut config = state.config.write().await;
            config.email.account.as_mut().unwrap().smtp_host = "localhost".to_owned();
            config.email.account.as_mut().unwrap().smtp_tls = mode;
            config.confirmation_provider.channels = vec![crate::config::ConfirmationChannel::Ntfy];
        }
        *state.email.test_root.write() = Some(certificate.as_ref().to_vec());
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        *state.hub_sender.lock().await = Some(sender);
        let confirmation_state = state.clone();
        let accepted = server.accepted.clone();
        let confirmation = tokio::spawn(async move {
            let message = receiver.recv().await.expect("confirmation request");
            let agentic_gpt_protocol::AgentMessage::ConfirmationRequest {
                request_id,
                payload,
                ..
            } = message
            else {
                panic!("expected confirmation request");
            };
            assert!(payload.reason.contains("Subject: Actual SMTP fixture"));
            assert!(payload.reason.contains("Approved body preview"));
            assert_eq!(
                accepted.load(std::sync::atomic::Ordering::Acquire),
                0,
                "SMTP connected before approval"
            );
            let sender = confirmation_state
                .pending_confirmations
                .lock()
                .await
                .remove(&request_id)
                .unwrap();
            sender.send("allow_once".to_owned()).unwrap();
        });
        let response=dispatch(&state,"room.email.send",json!({"to":["recipient@example.com"],"subject":"Actual SMTP fixture","text":"Approved body preview"})).await;
        confirmation.await.unwrap();
        let count = server.accepted.clone();
        let transcript = server.finish();
        assert_eq!(
            count.load(std::sync::atomic::Ordering::Acquire),
            1,
            "SMTP outcome caused an automatic retry"
        );
        assert!(transcript.iter().any(|line| line == "DATA accepted once"));
        assert_eq!(
            response["status"],
            if disconnect { "unknown" } else { "sent" },
            "{response}"
        );
        let id = response["id"].as_str().unwrap();
        assert_eq!(
            state.email.store.get(id).unwrap().status,
            if disconnect { "unknown" } else { "sent" }
        );
        let search = dispatch(
            &state,
            "room.email.search",
            json!({"query":"Approved body preview","field":"body"}),
        )
        .await;
        assert_eq!(search["items"][0]["ref"], id);
    }
}

// Tier ⑤: two independent services sharing the same private state reproduce
// multi-config Agents. Inactive opens and cancellation must not steal a live
// sender's recovery/cleanup ownership, and recovery is account-scoped.
#[test]
fn email_owner_lease_survives_inflight_guard_and_inactive_open_preserves_pending() {
    let fixture = Fixture::new();
    let first = fixture.open();
    let raw=b"From: sender@example.com\r\nTo: recipient@example.com\r\nSubject: Pending owner\r\n\r\nPending body";
    let create = |account| {
        first
            .store
            .create_send(
                account,
                raw,
                "Pending body",
                "sender@example.com",
                &["recipient@example.com".to_owned()],
                &[],
                "Pending owner",
            )
            .unwrap()
    };
    let first_id = create("account");
    let other_id = create("other-account");
    let temporary = fixture.root.join("state/email/.pending-inflight");
    std::fs::write(&temporary, b"inflight file").unwrap();
    let second =
        EmailService::open(&PrivateStatePaths::for_test(fixture.root.join("state"))).unwrap();
    assert_eq!(second.store.get(&first_id).unwrap().status, "pending");
    assert_eq!(second.store.get(&other_id).unwrap().status, "pending");
    assert!(
        temporary.exists(),
        "inactive startup cleaned an active owner's staging file"
    );
    assert!(second.activate("other-account").is_err());

    // Model async cancellation plus config disable while spawn_blocking still
    // holds its own cloned guard. The lease must remain exclusive.
    let in_flight = first.activate("account").unwrap();
    first.deactivate();
    drop(first);
    assert!(second.activate("other-account").is_err());
    drop(in_flight);
    let owner = second.activate("other-account").unwrap();
    assert_eq!(second.store.get(&other_id).unwrap().status, "unknown");
    assert_eq!(second.store.get(&first_id).unwrap().status, "pending");
    assert!(!temporary.exists());
    drop(owner);
}
