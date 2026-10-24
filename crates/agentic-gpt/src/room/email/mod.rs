//! The single-account email owner: durable mail facts, resumable synchronization,
//! and explicit IMAP/SMTP side effects. Event delivery is a retryable projection.
mod content;
mod network;
mod store;

use std::sync::Arc;

use agentic_gpt_protocol::{EventInjectRequest, EventSeverity};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{Config, EmailAccountConfig, EmailPolicy, ToolNamespace},
    private_state::PrivateStatePaths,
    state::AppState,
};

pub(crate) struct EmailService {
    store: store::Store,
    network: tokio::sync::Mutex<()>,
    database: std::path::PathBuf,
    owner: parking_lot::Mutex<Option<Arc<EmailOwner>>>,
    #[cfg(test)]
    test_root: parking_lot::RwLock<Option<Vec<u8>>>,
}

struct EmailOwner {
    account: String,
    _lease: crate::instance_lock::InstanceLock,
}

impl EmailService {
    pub(crate) fn open(paths: &PrivateStatePaths) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            store: store::Store::open(&paths.root)?,
            network: tokio::sync::Mutex::new(()),
            database: paths.root.join("email.sqlite3"),
            owner: parking_lot::Mutex::new(None),
            #[cfg(test)]
            test_root: parking_lot::RwLock::new(None),
        }))
    }

    fn activate(&self, account: &str) -> Result<Arc<EmailOwner>> {
        let mut current = self.owner.lock();
        if let Some(owner) = current.as_ref() {
            if owner.account != account {
                return Err(anyhow!("email_account_restart_required"));
            }
            return Ok(owner.clone());
        }
        let lease =
            crate::instance_lock::InstanceLock::acquire(&self.database, ".owner.lock", "email")
                .map_err(|_| anyhow!("email_owner_unavailable"))?;
        self.store.recover_owner(account)?;
        let owner = Arc::new(EmailOwner {
            account: account.to_owned(),
            _lease: lease,
        });
        *current = Some(owner.clone());
        Ok(owner)
    }

    fn deactivate(&self) {
        self.owner.lock().take();
    }
}

/// Called only by the offline CLI while it owns the Agent instance lock. The
/// caller must explicitly choose its notification behavior; old references stay
/// readable, but no guessed identity association authorizes remote mutations.
pub(crate) async fn recover(config: &Config, notify_existing: bool) -> Result<Value> {
    let account = config
        .email
        .account
        .clone()
        .ok_or_else(|| anyhow!("email_account_required"))?;
    let paths = crate::private_state::prepare()?;
    blocking(move || {
        let service = EmailService::open(&paths)?;
        let _owner = service.activate(&account_key(&account))?;
        let inbox = network::inbox(&account)?;
        service.store.recover_account(
            &account_key(&account),
            inbox.validity,
            inbox.baseline,
            notify_existing,
        )
    })
    .await
}

fn account_key(account: &EmailAccountConfig) -> String {
    let identity = format!(
        "{}\0{}\0{}",
        account.imap_host.to_ascii_lowercase(),
        account.imap_port,
        account.username
    );
    format!("{:x}", Sha256::digest(identity.as_bytes()))
}

fn error_value(error: impl std::fmt::Display) -> Value {
    let code = error.to_string();
    let code = if code.starts_with("email_") {
        code
    } else {
        "email_operation_failed".to_owned()
    };
    json!({"error":{"code":code,"message":code}})
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| anyhow!("email_worker_failed"))?
}

pub(crate) async fn run(state: AppState, shutdown: CancellationToken) -> Result<()> {
    loop {
        let config = state.config.read().await.clone();
        if config.email.enabled && config.toolsets.is_enabled(ToolNamespace::Room) {
            if let Some(account) = config.email.account.clone() {
                let key = account_key(&account);
                let active_service = state.email.clone();
                let active_key = key.clone();
                let owner = blocking(move || active_service.activate(&active_key)).await;
                let owner = match owner {
                    Ok(owner) => Some(owner),
                    Err(error) => {
                        let _ = state.email.store.sync_error(
                            &key,
                            error_value(error)["error"]["code"]
                                .as_str()
                                .unwrap_or("email_owner_unavailable"),
                        );
                        None
                    }
                };
                if owner.is_none() {
                    tokio::select! {_=shutdown.cancelled()=>return Ok(()),_=tokio::time::sleep(std::time::Duration::from_secs(1))=>{}}
                    continue;
                }
                // Serialize remote operations with sync so a stale flags snapshot
                // cannot overwrite a completed operation's local result.
                let lease = state.email.network.lock().await;
                let service = state.email.clone();
                let stopping = shutdown.clone();
                let network_owner = owner.clone();
                let result = blocking(move || {
                    let _owner = network_owner;
                    sync(&service, &account, &config, &stopping)
                })
                .await;
                drop(lease);
                if let Err(error) = result {
                    let code = error_value(error)["error"]["code"]
                        .as_str()
                        .unwrap_or("email_sync_failed")
                        .to_owned();
                    let _ = state.email.store.sync_error(&key, &code);
                    crate::utils::log_warn(format!("Email synchronization: {code}"));
                }
                if let Err(error) = deliver_events(&state).await {
                    crate::utils::log_warn(format!(
                        "Email reminder delivery will retry: {}",
                        error_value(error)["error"]["code"]
                            .as_str()
                            .unwrap_or("email_outbox_unavailable")
                    ));
                }
            }
        } else {
            state.email.deactivate();
        }
        if shutdown.is_cancelled() {
            return Ok(());
        }
        // Read the current interval repeatedly so live changes take effect even
        // while the old interval is waiting. No missed interval triggers a burst.
        let started = tokio::time::Instant::now();
        loop {
            tokio::select! {
                _=shutdown.cancelled()=>return Ok(()),
                _=tokio::time::sleep(std::time::Duration::from_secs(1))=>{},
            }
            let waiting_config = state.config.read().await;
            if !waiting_config.email.enabled
                || !waiting_config.toolsets.is_enabled(ToolNamespace::Room)
            {
                state.email.deactivate();
                break;
            }
            let seconds = waiting_config.email.sync_interval_seconds;
            if started.elapsed().as_secs() >= seconds {
                break;
            }
        }
    }
}

fn sync(
    service: &EmailService,
    account: &EmailAccountConfig,
    config: &Config,
    shutdown: &CancellationToken,
) -> Result<()> {
    let key = account_key(account);
    let mut inbox = network::inbox(account)?;
    let record = service
        .store
        .sync_record(&key, inbox.validity, inbox.baseline)?;
    let uids = inbox.uids.clone();
    let mut since_cleanup = 0usize;
    let mut bytes_since_cleanup = 0u64;
    let cleanup_bytes = (config.email.max_storage_bytes / 8).clamp(1, 16 * 1024 * 1024);
    service
        .store
        .cleanup(config.email.retention_days, config.email.max_storage_bytes)?;
    for uid in uids.iter().copied().filter(|uid| *uid > record.last_uid) {
        if shutdown.is_cancelled() {
            return Ok(());
        }
        if let Some(message) = inbox.message(uid)? {
            service.store.save_received(&key, &record, &message)?;
            bytes_since_cleanup = bytes_since_cleanup.saturating_add(message.raw.len() as u64);
        } else {
            service.store.skip_expunged(&key, uid)?;
        }
        since_cleanup += 1;
        if since_cleanup >= 25 || bytes_since_cleanup >= cleanup_bytes {
            service
                .store
                .cleanup(config.email.retention_days, config.email.max_storage_bytes)?;
            since_cleanup = 0;
            bytes_since_cleanup = 0;
        }
    }
    let mut flags = Vec::with_capacity(uids.len());
    for chunk in uids.chunks(network::FETCH_BATCH) {
        if shutdown.is_cancelled() {
            return Ok(());
        }
        flags.extend(inbox.flags(chunk)?);
    }
    service.store.finish_sync(&key, &flags)?;
    service
        .store
        .cleanup(config.email.retention_days, config.email.max_storage_bytes)?;
    Ok(())
}

async fn deliver_events(state: &AppState) -> Result<()> {
    let service = state.email.clone();
    let events = blocking(move || service.store.pending_events()).await?;
    let ttl = state.config.read().await.events.low_ttl_seconds;
    for (id, message) in events {
        let request = EventInjectRequest {
            message,
            severity: Some(EventSeverity::Medium),
            reference: id.clone(),
        };
        // Full EventStore leaves mail/outbox/progress untouched. Injection uses
        // the stable mail ID, so a crash before ACK cannot duplicate the event.
        if state
            .event_store
            .inject_external_once(&format!("email:{id}"), &request, ttl)
            .is_err()
        {
            break;
        }
        let service = state.email.clone();
        blocking(move || service.store.acknowledge_event(&id)).await?;
    }
    Ok(())
}

pub(crate) async fn dispatch(state: &AppState, operation: &str, args: Value) -> Value {
    let config = state.config.read().await.clone();
    if !config.email.enabled {
        return error_value("email_disabled");
    }
    let Some(account) = config.email.account.clone() else {
        return error_value("email_account_required");
    };
    let service = state.email.clone();
    let key = account_key(&account);
    let _owner = match blocking(move || service.activate(&key)).await {
        Ok(owner) => owner,
        Err(error) => return error_value(error),
    };
    let result = match operation {
        "room.email.list" => list(state, &account, args).await,
        "room.email.read" => read(state, &account, args).await,
        "room.email.search" => search(state, &account, args).await,
        "room.email.attachment" => attachment(state, &account, args).await,
        "room.email.send" => send(state, &config, &account, &_owner, args).await,
        "room.email.operate" => operate(state, &config, &account, &_owner, args).await,
        _ => Err(anyhow!("email_operation_unknown")),
    };
    match result {
        Ok(value) => {
            let serialized_bytes = match serde_json::to_vec(&value) {
                Ok(bytes) => bytes.len(),
                Err(_) => return error_value("email_response_serialization_failed"),
            };
            let image_bytes = value
                .get("images")
                .and_then(Value::as_array)
                .map(|images| {
                    images
                        .iter()
                        .filter_map(|image| image.get("data").and_then(Value::as_str))
                        .map(str::len)
                        .sum::<usize>()
                })
                .unwrap_or(0);
            // MCP emits metadata both as text and structured content. Image
            // payloads become image blocks, so their Base64 is counted once.
            let projected_bytes = serialized_bytes
                .saturating_mul(2)
                .saturating_sub(image_bytes);
            if projected_bytes > 8 * 1024 * 1024 {
                error_value("email_response_too_large")
            } else {
                value
            }
        }
        Err(error) => error_value(error),
    }
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListFilter {
    unread: Option<bool>,
    received_after: Option<String>,
    received_before: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListRequest {
    cursor: Option<String>,
    limit: Option<usize>,
    #[serde(default)]
    filter: ListFilter,
}

#[derive(Serialize, Deserialize)]
struct ListCursor {
    v: u8,
    account: String,
    filter: ListFilter,
    time: i64,
    id: String,
}

fn parse_date(date: &Option<String>) -> Result<Option<i64>> {
    date.as_ref()
        .map(|date| {
            chrono::DateTime::parse_from_rfc3339(date)
                .map(|date| date.timestamp_millis())
                .map_err(|_| anyhow!("email_date_invalid"))
        })
        .transpose()
}

async fn list(state: &AppState, account: &EmailAccountConfig, args: Value) -> Result<Value> {
    let request: ListRequest =
        serde_json::from_value(args).map_err(|_| anyhow!("email_arguments_invalid"))?;
    let limit = request.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err(anyhow!("email_limit_invalid"));
    }
    let after = parse_date(&request.filter.received_after)?;
    let before = parse_date(&request.filter.received_before)?;
    if after.zip(before).is_some_and(|(a, b)| a >= b) {
        return Err(anyhow!("email_date_range_invalid"));
    }
    let key = account_key(account);
    let cursor: Option<ListCursor> = request.cursor.as_deref().map(decode_cursor).transpose()?;
    if cursor.as_ref().is_some_and(|cursor| {
        cursor.v != 1 || cursor.account != key || cursor.filter != request.filter
    }) {
        return Err(anyhow!("email_cursor_filter_mismatch"));
    }
    let service = state.email.clone();
    blocking(move||{
        let mut mails=service.store.list_page(&key,request.filter.unread,after,before,cursor.as_ref().map(|c|(c.time,c.id.as_str())),limit+1)?;
        let has_more=mails.len()>limit;
        mails.truncate(limit);
        let mut response=json!({"items":mails.iter().map(store::Mail::summary).collect::<Vec<_>>(),"hasMore":has_more,"sync":service.store.sync_status(&key)?});
        if has_more {
            if let Some(last)=mails.last() { response["nextCursor"]=json!(encode_cursor(&ListCursor{v:1,account:key,filter:request.filter,time:last.received_at,id:last.id.clone()})?); }
        }
        Ok(response)
    }).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadRequest {
    id: String,
    view: Option<String>,
    cursor: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct TextCursor {
    v: u8,
    id: String,
    view: String,
    revision: String,
    offset: usize,
}

fn text_cursor(mail: &store::Mail, offset: usize) -> Result<String> {
    encode_cursor(&TextCursor {
        v: 1,
        id: mail.id.clone(),
        view: "text".to_owned(),
        revision: body_revision(mail),
        offset,
    })
}
fn body_revision(mail: &store::Mail) -> String {
    format!(
        "{:x}",
        Sha256::digest(mail.body.as_deref().unwrap_or_default().as_bytes())
    )
}

fn mail_metadata(mail: &store::Mail, content: &content::Content) -> Value {
    json!({"id":mail.id,"from":mail.from,"to":content::addresses(&mail.to),"cc":content::addresses(&mail.cc),"subject":mail.subject,
        "receivedAt":store::timestamp(mail.received_at),"status":mail.status,
        "attachments":content.attachments,"hasMore":false})
}

async fn read(state: &AppState, account: &EmailAccountConfig, args: Value) -> Result<Value> {
    let request: ReadRequest =
        serde_json::from_value(args).map_err(|_| anyhow!("email_arguments_invalid"))?;
    let view = request.view.as_deref().unwrap_or("text");
    if !matches!(view, "text" | "image" | "imageWithRemote") {
        return Err(anyhow!("email_view_invalid"));
    }
    let service = state.email.clone();
    let key = account_key(account);
    let id = request.id.clone();
    let (mail, content) = blocking(move || {
        let mail = owned_mail(&service, &key, &id)?;
        let content = service.store.read_content(&mail)?;
        Ok((mail, content))
    })
    .await?;
    let mut response = mail_metadata(&mail, &content);
    if view != "text" {
        let remote = view == "imageWithRemote";
        let mut html = content::render_html(&content);
        if remote {
            html = crate::email_remote::prepare(&mail.id, &html, request.cursor.is_some())
                .await
                .map_err(anyhow::Error::msg)?;
        }
        let rendered =
            crate::email_render::render(&mail.id, &html, request.cursor.as_deref(), remote)
                .await
                .map_err(anyhow::Error::msg)?;
        let object = rendered
            .as_object()
            .ok_or_else(|| anyhow!("email_renderer_response_invalid"))?;
        for (key, value) in object {
            response[key] = value.clone();
        }
    } else {
        if mail.format == "conversionFailed" {
            return Err(anyhow!("email_text_conversion_failed_use_image_view"));
        }
        let body = mail
            .body
            .as_deref()
            .ok_or_else(|| anyhow!("email_content_expired"))?;
        let offset = if let Some(cursor) = request.cursor.as_deref() {
            let cursor: TextCursor = decode_cursor(cursor)?;
            if cursor.v != 1
                || cursor.id != mail.id
                || cursor.view != "text"
                || cursor.revision != body_revision(&mail)
                || cursor.offset > body.len()
                || !body.is_char_boundary(cursor.offset)
            {
                return Err(anyhow!("email_cursor_invalid"));
            }
            cursor.offset
        } else {
            0
        };
        let end = content::end_boundary(body, offset, content::TEXT_BUDGET);
        response["body"] = json!(&body[offset..end]);
        response["format"] = json!(mail.format);
        response["hasMore"] = json!(end < body.len());
        if end < body.len() {
            response["nextCursor"] = json!(text_cursor(&mail, end)?);
        }
    }
    Ok(response)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchRequest {
    query: String,
    id: Option<String>,
    field: Option<String>,
}

async fn search(state: &AppState, account: &EmailAccountConfig, args: Value) -> Result<Value> {
    let request: SearchRequest =
        serde_json::from_value(args).map_err(|_| anyhow!("email_arguments_invalid"))?;
    if request.query.trim().is_empty() || request.query.len() > 4096 {
        return Err(anyhow!("email_query_invalid"));
    }
    let service = state.email.clone();
    let key = account_key(account);
    blocking(move || {
        let (items, truncated) = if let Some(id) = request.id {
            let mail = owned_mail(&service, &key, &id)?;
            let body = mail
                .body
                .as_deref()
                .ok_or_else(|| anyhow!("email_content_expired"))?;
            let pattern = regex::RegexBuilder::new(&regex::escape(&request.query))
                .case_insensitive(true)
                .build()
                .map_err(|_| anyhow!("email_query_invalid"))?;
            let mut items = Vec::new();
            let mut truncated = false;
            let mut bytes = 0;
            for hit in pattern.find_iter(body) {
                let mut start = hit.start().saturating_sub(120);
                while !body.is_char_boundary(start) {
                    start += 1;
                }
                let mut end = (hit.end() + 160).min(body.len());
                while !body.is_char_boundary(end) {
                    end -= 1;
                }
                let text = &body[start..end];
                let cursor = text_cursor(&mail, start)?;
                bytes += text.len() + cursor.len() + 40;
                if items.len() >= 100 || bytes > content::TEXT_BUDGET {
                    truncated = true;
                    break;
                }
                items.push(json!({"ref":cursor,"text":text}));
            }
            (items, truncated)
        } else {
            let field = request.field.as_deref().unwrap_or("all");
            if !matches!(field, "all" | "subject" | "body" | "from") {
                return Err(anyhow!("email_field_invalid"));
            }
            service.store.search(&key, &request.query, field)?
        };
        Ok(json!({"items":items,"truncated":truncated,"sync":service.store.sync_status(&key)?}))
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AttachmentRequest {
    id: String,
    attachment_id: String,
    download: Option<bool>,
    path: Option<String>,
}

async fn attachment(state: &AppState, account: &EmailAccountConfig, args: Value) -> Result<Value> {
    let request: AttachmentRequest =
        serde_json::from_value(args).map_err(|_| anyhow!("email_arguments_invalid"))?;
    let service = state.email.clone();
    let key = account_key(account);
    let id = request.id.clone();
    let attachment_id = request.attachment_id.clone();
    let resource = blocking(move || {
        let mail = owned_mail(&service, &key, &id)?;
        let content = service.store.read_content(&mail)?;
        content
            .attachments
            .into_iter()
            .find(|attachment| attachment.id == attachment_id)
            .ok_or_else(|| anyhow!("email_attachment_not_found"))
    })
    .await?;
    if request.download.unwrap_or(false) || request.path.is_some() {
        Ok(crate::email_files::save_attachment(
            state,
            &request.id,
            &resource.name,
            request.path.as_deref(),
            &resource.bytes,
        )
        .await)
    } else {
        Ok(crate::email_files::read_attachment(
            state,
            &resource.name,
            &resource.mime_type,
            &resource.bytes,
        )
        .await)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SendRequest {
    to: Vec<String>,
    #[serde(default)]
    cc: Vec<String>,
    #[serde(default)]
    bcc: Vec<String>,
    subject: String,
    text: String,
    reply_to_id: Option<String>,
}

async fn send(
    state: &AppState,
    config: &Config,
    account: &EmailAccountConfig,
    owner: &Arc<EmailOwner>,
    args: Value,
) -> Result<Value> {
    let request: SendRequest =
        serde_json::from_value(args).map_err(|_| anyhow!("email_arguments_invalid"))?;
    if request.to.is_empty()
        || request.to.len() + request.cc.len() + request.bcc.len() > 100
        || request.subject.len() > 4096
        || request.text.len() > 1024 * 1024
    {
        return Err(anyhow!("email_send_limits_exceeded"));
    }
    if request.subject.contains(['\r', '\n']) {
        return Err(anyhow!("email_subject_invalid"));
    }
    if request
        .to
        .iter()
        .chain(&request.cc)
        .chain(&request.bcc)
        .any(|address| address.len() > 1024 || address.contains(['\r', '\n']))
    {
        return Err(anyhow!("email_recipient_invalid"));
    }
    if config.email.send_policy == EmailPolicy::Deny {
        return Err(anyhow!("email_policy_denied"));
    }
    let key = account_key(account);
    let reply = if let Some(reply_id) = &request.reply_to_id {
        let original = owned_mail(&state.email, &key, reply_id)?;
        let message_id = original
            .message_id
            .ok_or_else(|| anyhow!("email_reply_message_id_unavailable"))?;
        let references = match original.references {
            Some(references) => format!("{references} {message_id}"),
            None => message_id.clone(),
        };
        Some((message_id, references))
    } else {
        None
    };
    let message = network::build_message(
        account,
        &request.to,
        &request.cc,
        &request.bcc,
        &request.subject,
        &request.text,
        reply,
    )?;
    let raw = message.formatted();
    let id = state.email.store.create_send(
        &key,
        &raw,
        &content::normalize_text(&request.text),
        &account.from,
        &request.to,
        &request.cc,
        &request.subject,
    )?;
    let mut preview = format!(
        "To ({} recipients): {}\nSubject: {}",
        request.to.len(),
        crate::confirmation::short_confirmation_text(&request.to.join(", "), 700),
        crate::confirmation::short_confirmation_text(&request.subject, 400)
    );
    if !request.cc.is_empty() {
        preview.push_str(&format!(
            "\nCc ({} recipients): {}",
            request.cc.len(),
            crate::confirmation::short_confirmation_text(&request.cc.join(", "), 700)
        ));
    }
    if !request.bcc.is_empty() {
        preview.push_str(&format!(
            "\nBcc ({} recipients): {}",
            request.bcc.len(),
            crate::confirmation::short_confirmation_text(&request.bcc.join(", "), 700)
        ));
    }
    preview.push_str(&format!(
        "\n\n{}",
        request.text.chars().take(800).collect::<String>()
    ));
    if let Err(error) = confirm(
        state,
        config,
        config.email.send_policy,
        "room.email.send",
        &preview,
    )
    .await
    {
        let _ = state.email.store.sent_status(&id, "denied");
        let mut failure = error_value(error);
        failure["id"] = json!(id);
        failure["status"] = json!("denied");
        return Ok(failure);
    }
    let account = account.clone();
    let service = state.email.clone();
    let send_id = id.clone();
    let retention_days = config.email.retention_days;
    let max_storage_bytes = config.email.max_storage_bytes;
    let network_owner = owner.clone();
    let outcome = blocking(move || {
        let _owner = network_owner;
        #[cfg(test)]
        let result = match service.test_root.read().as_deref() {
            Some(root) => network::send_with_test_root(&account, &message, root),
            None => network::send(&account, &message),
        };
        #[cfg(not(test))]
        let result = network::send(&account, &message);
        let status = match &result {
            Ok(network::SendOutcome::Sent) => "sent",
            Ok(network::SendOutcome::Unknown) => "unknown",
            _ => "failed",
        };
        let saved = service.store.sent_status(&send_id, status);
        // Cleanup errors cannot turn a successful external send into a request
        // that looks safe to retry. The next sync retries content cleanup.
        if saved.is_ok() {
            let _ = service.store.cleanup(retention_days, max_storage_bytes);
        }
        Ok((result, saved))
    })
    .await;
    let outcome = match outcome {
        Ok((result, Ok(()))) => match result {
            Ok(outcome) => outcome,
            Err(error) => return Ok(send_failure(error, &id, "failed")),
        },
        Ok((_, Err(_))) => {
            return Ok(send_failure(
                "email_send_outcome_persistence_failed",
                &id,
                "unknown",
            ))
        }
        Err(error) => return Ok(send_failure(error, &id, "unknown")),
    };
    match outcome {
        network::SendOutcome::Sent => Ok(json!({"id":id,"status":"sent"})),
        network::SendOutcome::Unknown => Ok(json!({"id":id,"status":"unknown"})),
        network::SendOutcome::Failed => {
            let mut failure = error_value("email_send_failed");
            failure["id"] = json!(id);
            failure["status"] = json!("failed");
            Ok(failure)
        }
    }
}

fn send_failure(error: impl std::fmt::Display, id: &str, status: &str) -> Value {
    let mut failure = error_value(error);
    failure["id"] = json!(id);
    failure["status"] = json!(status);
    failure
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperateRequest {
    id: String,
    action: String,
}

async fn operate(
    state: &AppState,
    config: &Config,
    account: &EmailAccountConfig,
    owner: &Arc<EmailOwner>,
    args: Value,
) -> Result<Value> {
    let request: OperateRequest =
        serde_json::from_value(args).map_err(|_| anyhow!("email_arguments_invalid"))?;
    if !matches!(
        request.action.as_str(),
        "markRead" | "markUnread" | "flag" | "unflag" | "trash"
    ) {
        return Err(anyhow!("email_action_invalid"));
    }
    let key = account_key(account);
    let mail = owned_mail(&state.email, &key, &request.id)?;
    let validity = mail
        .validity
        .ok_or_else(|| anyhow!("email_remote_operation_unsupported"))?;
    let uid = mail
        .uid
        .ok_or_else(|| anyhow!("email_remote_operation_unsupported"))?;
    if !mail.inbox {
        return Err(anyhow!("email_remote_message_unavailable"));
    }
    let status = state.email.store.sync_status(&key)?;
    if status["state"] == "recoveryRequired" {
        return Err(anyhow!("email_identity_recovery_required"));
    }
    if request.action == "trash" {
        confirm(
            state,
            config,
            config.email.trash_policy,
            "room.email.operate",
            &format!(
                "Move email to Trash\nFrom: {}\nSubject: {}",
                crate::confirmation::short_confirmation_text(&mail.from, 700),
                crate::confirmation::short_confirmation_text(&mail.subject, 800)
            ),
        )
        .await?;
    }
    let _lease = state.email.network.lock().await;
    let service = state.email.clone();
    let account = account.clone();
    let network_owner = owner.clone();
    blocking(move || {
        let _owner = network_owner;
        network::operate(&account, validity, uid, &request.action)?;
        service
            .store
            .apply_operation(&request.id, &request.action)?;
        Ok(json!({"id":request.id,"action":request.action,"status":"applied"}))
    })
    .await
}

async fn confirm(
    state: &AppState,
    config: &Config,
    policy: EmailPolicy,
    operation: &str,
    preview: &str,
) -> Result<()> {
    let decision = match policy {
        EmailPolicy::Allow => crate::policy::PolicyDecision::Allow,
        EmailPolicy::Confirm => crate::policy::PolicyDecision::Confirm,
        EmailPolicy::Deny => return Err(anyhow!("email_policy_denied")),
    };
    crate::confirmation::authorize_email_operation(state, config, decision, operation, preview)
        .await
        .map_err(|reason| anyhow!("email_confirmation_{reason}"))?;
    Ok(())
}

fn owned_mail(service: &EmailService, account: &str, id: &str) -> Result<store::Mail> {
    let mail = service.store.get(id)?;
    if mail.account != account {
        return Err(anyhow!("email_not_found"));
    }
    Ok(mail)
}

fn encode_cursor(value: &impl Serialize) -> Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(value)?))
}
fn decode_cursor<T: serde::de::DeserializeOwned>(cursor: &str) -> Result<T> {
    if cursor.len() > 8192 {
        return Err(anyhow!("email_cursor_invalid"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| anyhow!("email_cursor_invalid"))?;
    serde_json::from_slice(&bytes).map_err(|_| anyhow!("email_cursor_invalid"))
}

#[cfg(test)]
mod tests;
