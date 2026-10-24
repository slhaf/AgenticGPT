use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use uuid::Uuid;

use super::{content, network};

pub(super) struct Store {
    conn: Mutex<Connection>,
    raw_root: PathBuf,
    orphan_cleanup_needed: AtomicBool,
}

#[derive(Clone)]
pub(super) struct Mail {
    pub id: String,
    pub account: String,
    pub validity: Option<u32>,
    pub uid: Option<u32>,
    pub inbox: bool,
    pub from: String,
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub received_at: i64,
    pub unread: bool,
    pub status: String,
    pub snippet: String,
    pub body: Option<String>,
    pub format: String,
    pub message_id: Option<String>,
    pub references: Option<String>,
    pub raw_bytes: u64,
}

impl Mail {
    pub fn summary(&self) -> Value {
        json!({"id":self.id,"from":self.from,"subject":self.subject,
            "receivedAt":timestamp(self.received_at),"unread":self.unread,"snippet":self.snippet})
    }
}

pub(super) struct SyncRecord {
    pub validity: u32,
    pub baseline: u32,
    pub last_uid: u32,
    pub recovery: bool,
}

const MAIL_COLUMNS: &str = "id,account,validity,uid,inbox,sender,recipients,cc,subject,received_at,unread,status,snippet,body,format,message_id,refs,raw_bytes";

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        private_directory(root)?;
        let raw_root = root.join("email");
        private_directory(&raw_root)?;
        let database = root.join("email.sqlite3");
        if let Ok(metadata) = fs::symlink_metadata(&database) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(anyhow!("email_database_path_invalid"));
            }
        }
        let mut conn =
            Connection::open(&database).map_err(|_| anyhow!("email_database_open_failed"))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&database, fs::Permissions::from_mode(0o600))?;
        }
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if !matches!(version, 0..=2) {
            return Err(anyhow!("email_database_version_unsupported"));
        }
        conn.execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
            PRAGMA auto_vacuum=INCREMENTAL; PRAGMA secure_delete=ON;
            CREATE TABLE IF NOT EXISTS accounts (
                account TEXT PRIMARY KEY, validity INTEGER NOT NULL, baseline INTEGER NOT NULL,
                last_uid INTEGER NOT NULL DEFAULT 0, initial_complete INTEGER NOT NULL DEFAULT 0,
                recovery INTEGER NOT NULL DEFAULT 0, last_success INTEGER, last_error TEXT
            );
            CREATE TABLE IF NOT EXISTS mail (
                id TEXT PRIMARY KEY, account TEXT NOT NULL, validity INTEGER, uid INTEGER,
                inbox INTEGER NOT NULL, sender TEXT NOT NULL, recipients TEXT NOT NULL,
                cc TEXT NOT NULL, subject TEXT NOT NULL, received_at INTEGER NOT NULL,
                unread INTEGER NOT NULL, flagged INTEGER NOT NULL DEFAULT 0, status TEXT NOT NULL,
                snippet TEXT NOT NULL, body TEXT, format TEXT NOT NULL, message_id TEXT, refs TEXT,
                raw_bytes INTEGER NOT NULL DEFAULT 0, body_bytes INTEGER NOT NULL DEFAULT 0,
                UNIQUE(account, validity, uid)
            );
            CREATE INDEX IF NOT EXISTS email_inbox ON mail(account,inbox,received_at DESC,id DESC);
            CREATE TABLE IF NOT EXISTS outbox (id TEXT PRIMARY KEY, message TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS stats (key TEXT PRIMARY KEY,value INTEGER NOT NULL);
            INSERT OR IGNORE INTO stats(key,value) SELECT 'content_bytes',COALESCE(SUM(raw_bytes+body_bytes),0) FROM mail;",
        )?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 2 {
            tx.execute_batch(
                "ALTER TABLE mail ADD COLUMN attachments TEXT; PRAGMA user_version=2;",
            )?;
        }
        tx.commit()?;
        let store = Self {
            conn: Mutex::new(conn),
            raw_root,
            orphan_cleanup_needed: AtomicBool::new(false),
        };
        Ok(store)
    }

    /// The caller holds the database-wide email owner lease. Merely opening an
    /// inactive Agent must never recover another running sender's records.
    pub fn recover_owner(&self, account: &str) -> Result<()> {
        self.conn.lock().execute(
            "UPDATE mail SET status='unknown' WHERE account=? AND status='pending'",
            [account],
        )?;
        self.remove_orphans()
    }

    fn remove_orphans(&self) -> Result<()> {
        let conn = self.conn.lock();
        for entry in fs::read_dir(&self.raw_root)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.starts_with(".pending-") {
                fs::remove_file(entry.path())?;
            } else if let Some(id) = name.strip_suffix(".eml") {
                let exists: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM mail WHERE id=? AND raw_bytes>0)",
                    [id],
                    |r| r.get(0),
                )?;
                if !exists {
                    fs::remove_file(entry.path())?;
                }
            }
        }
        Ok(())
    }

    pub fn sync_record(&self, account: &str, validity: u32, baseline: u32) -> Result<SyncRecord> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR IGNORE INTO accounts(account,validity,baseline) VALUES(?,?,?)",
            params![account, validity, baseline],
        )?;
        conn.execute(
            "UPDATE accounts SET validity=?,baseline=? WHERE account=? AND validity=0",
            params![validity, baseline, account],
        )?;
        let record = conn.query_row(
            "SELECT validity,baseline,last_uid,recovery FROM accounts WHERE account=?",
            [account],
            |r| {
                Ok(SyncRecord {
                    validity: r.get(0)?,
                    baseline: r.get(1)?,
                    last_uid: r.get(2)?,
                    recovery: r.get(3)?,
                })
            },
        )?;
        if record.validity != validity {
            conn.execute("UPDATE accounts SET recovery=1,last_error='email_identity_recovery_required' WHERE account=?", [account])?;
            return Err(anyhow!("email_identity_recovery_required"));
        }
        if record.recovery {
            return Err(anyhow!("email_identity_recovery_required"));
        }
        Ok(record)
    }

    pub fn sync_status(&self, account: &str) -> Result<Value> {
        let conn = self.conn.lock();
        let data = conn.query_row("SELECT initial_complete,recovery,last_success,last_error FROM accounts WHERE account=?", [account], |r| {
            Ok((r.get::<_,bool>(0)?,r.get::<_,bool>(1)?,r.get::<_,Option<i64>>(2)?,r.get::<_,Option<String>>(3)?))
        }).optional()?;
        let Some((complete, recovery, last_success, last_error)) = data else {
            return Ok(json!({"state":"syncing","initialSyncComplete":false}));
        };
        let mut status = json!({"state":if recovery {"recoveryRequired"} else if last_error.is_some() {"error"} else if complete {"ready"} else {"syncing"},"initialSyncComplete":complete});
        if let Some(value) = last_success {
            status["lastSuccessfulSyncAt"] = json!(timestamp(value));
        }
        if let Some(value) = last_error {
            status["lastError"] = json!(value);
        }
        Ok(status)
    }

    pub fn sync_error(&self, account: &str, message: &str) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO accounts(account,validity,baseline,last_error) VALUES(?,0,0,?) ON CONFLICT(account) DO UPDATE SET last_error=excluded.last_error",
            params![account, message],
        )?;
        Ok(())
    }

    pub fn recover_account(
        &self,
        account: &str,
        validity: u32,
        baseline: u32,
        notify_existing: bool,
    ) -> Result<Value> {
        let mut conn = self.conn.lock();
        let previous: Option<(u32, bool)> = conn
            .query_row(
                "SELECT validity,recovery FROM accounts WHERE account=?",
                [account],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((old_validity, recovery)) = previous else {
            return Err(anyhow!("email_recovery_not_required"));
        };
        if old_validity == 0 || (old_validity == validity && !recovery) {
            return Err(anyhow!("email_recovery_not_required"));
        }
        let tx = conn.transaction()?;
        let detached = tx.execute(
            "UPDATE mail SET inbox=0,validity=NULL,uid=NULL WHERE account=? AND status='received'",
            [account],
        )?;
        let pending: i64 = tx.query_row(
            "SELECT COUNT(*) FROM outbox WHERE id IN (SELECT id FROM mail WHERE account=?)",
            [account],
            |row| row.get(0),
        )?;
        tx.execute("UPDATE accounts SET validity=?,baseline=?,last_uid=0,initial_complete=0,recovery=0,last_success=NULL,last_error=NULL WHERE account=?",params![validity,if notify_existing{0}else{baseline},account])?;
        tx.commit()?;
        Ok(
            json!({"status":"recoveryScheduled","preservedHistoricalCopies":detached,"retainedPendingNotifications":pending,"notifyExisting":notify_existing,"newMailIdsOnImport":true,"searchMayIncludeHistoricalCopies":true}),
        )
    }

    pub fn finish_sync(&self, account: &str, current: &[network::RemoteFlags]) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE mail SET inbox=0 WHERE account=? AND status='received'",
            [account],
        )?;
        for mail in current {
            tx.execute("UPDATE mail SET inbox=1,unread=?,flagged=? WHERE account=? AND uid=? AND validity=(SELECT validity FROM accounts WHERE account=?)", params![mail.unread,mail.flagged,account,mail.uid,account])?;
        }
        tx.execute(
            "UPDATE accounts SET initial_complete=1,last_success=?,last_error=NULL WHERE account=?",
            params![Utc::now().timestamp_millis(), account],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn save_received(
        &self,
        account: &str,
        record: &SyncRecord,
        remote: &network::RemoteMessage,
    ) -> Result<()> {
        let (content, unavailable) = if let Some(reason) = remote.unavailable {
            (content::unavailable_metadata(&remote.raw), Some(reason))
        } else {
            match content::parse(&remote.raw) {
                Ok(content) => (content, None),
                Err(_) => (content::unavailable_metadata(&remote.raw), Some("invalid")),
            }
        };
        let id = format!("mail_{}", Uuid::new_v4().simple());
        let raw_bytes = if remote.unavailable.is_some() {
            0
        } else {
            remote.raw.len()
        };
        if raw_bytes > 0 {
            self.write_raw(&id, &remote.raw)?;
        }
        let body = unavailable.is_none().then_some(content.body.as_str());
        let snippet = match unavailable {
            Some("tooLarge") => "Content unavailable: message exceeds the 32 MiB limit".to_owned(),
            Some(_) => "Content unavailable: MIME parsing failed".to_owned(),
            None => snippet(&content.body),
        };
        let format = unavailable.unwrap_or(content.format);
        let attachments = unavailable
            .is_none()
            .then(|| serde_json::to_string(&content.attachments))
            .transpose()?;
        let body_bytes = body.map_or(0, str::len) + attachments.as_ref().map_or(0, String::len);
        let result = (|| -> Result<()> {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            tx.execute("INSERT INTO mail(id,account,validity,uid,inbox,sender,recipients,cc,subject,received_at,unread,flagged,status,snippet,body,format,message_id,refs,raw_bytes,body_bytes,attachments)
                VALUES(?,?,?,?,1,?,?,?,?,?,?,?,'received',?,?,?,?,?,?,?,?)", params![
                id,account,record.validity,remote.uid,content.from,content.to,content.cc,content.subject,
                remote.received_at.timestamp_millis(),remote.unread,remote.flagged,
                snippet,body,format,content.message_id,content.references,raw_bytes as i64,body_bytes as i64,attachments])?;
            if remote.uid > record.baseline {
                let notice = format!(
                    "New email from {}: {}. Read with room.email.read id={id}",
                    bounded(&content.from, 120),
                    bounded(&content.subject, 200)
                );
                tx.execute(
                    "INSERT INTO outbox(id,message) VALUES(?,?)",
                    params![id, notice],
                )?;
            }
            tx.execute(
                "UPDATE accounts SET last_uid=MAX(last_uid,?) WHERE account=?",
                params![remote.uid, account],
            )?;
            tx.execute(
                "UPDATE stats SET value=value+? WHERE key='content_bytes'",
                [(raw_bytes + body_bytes) as i64],
            )?;
            tx.commit()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(self.raw_root.join(format!("{id}.eml")));
        }
        result
    }

    pub fn skip_expunged(&self, account: &str, uid: u32) -> Result<()> {
        self.conn.lock().execute(
            "UPDATE accounts SET last_uid=MAX(last_uid,?) WHERE account=?",
            params![uid, account],
        )?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Mail> {
        let conn = self.conn.lock();
        let query = format!("SELECT {MAIL_COLUMNS} FROM mail WHERE id=?");
        conn.query_row(&query, [id], row_mail)
            .optional()?
            .ok_or_else(|| anyhow!("email_not_found"))
    }

    pub fn list_page(
        &self,
        account: &str,
        unread: Option<bool>,
        after: Option<i64>,
        before: Option<i64>,
        key: Option<(i64, &str)>,
        limit: usize,
    ) -> Result<Vec<Mail>> {
        let conn = self.conn.lock();
        let columns = MAIL_COLUMNS.replace(",body,", ",NULL,");
        let query = format!("SELECT {columns} FROM mail WHERE account=?1 AND inbox=1 AND status='received'
            AND (?2 IS NULL OR unread=?2) AND (?3 IS NULL OR received_at>=?3) AND (?4 IS NULL OR received_at<?4)
            AND (?5 IS NULL OR received_at<?5 OR (received_at=?5 AND id<?6))
            ORDER BY received_at DESC,id DESC LIMIT ?7");
        let mut statement = conn.prepare(&query)?;
        let rows = statement.query_map(
            params![
                account,
                unread,
                after,
                before,
                key.map(|k| k.0),
                key.map(|k| k.1),
                limit as i64
            ],
            row_mail,
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn search(&self, account: &str, query: &str, field: &str) -> Result<(Vec<Value>, bool)> {
        let conn = self.conn.lock();
        let mut statement = conn.prepare("SELECT id,subject,sender,body FROM mail WHERE account=? AND status IN ('received','sent','unknown','pending') ORDER BY received_at DESC,id DESC")?;
        let mut rows = statement.query([account])?;
        let query = query.to_lowercase();
        let mut items = Vec::new();
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let subject: String = row.get(1)?;
            let sender: String = row.get(2)?;
            let body: Option<String> = row.get(3)?;
            let matched = match field {
                "subject" => subject.to_lowercase().contains(&query),
                "from" => sender.to_lowercase().contains(&query),
                "body" => body
                    .as_ref()
                    .is_some_and(|body| body.to_lowercase().contains(&query)),
                _ => {
                    subject.to_lowercase().contains(&query)
                        || sender.to_lowercase().contains(&query)
                        || body
                            .as_ref()
                            .is_some_and(|body| body.to_lowercase().contains(&query))
                }
            };
            if matched {
                let id: String = row.get(0)?;
                bytes += subject.len() + id.len() + 40;
                if items.len() >= 100 || bytes > content::TEXT_BUDGET {
                    return Ok((items, true));
                }
                items.push(json!({"ref":id,"text":subject}));
            }
        }
        Ok((items, false))
    }

    fn check_content(mail: &Mail) -> Result<()> {
        if mail.format == "tooLarge" {
            return Err(anyhow!("email_message_too_large"));
        }
        if mail.format == "invalid" {
            return Err(anyhow!("email_mime_invalid"));
        }
        if mail.body.is_none() || mail.raw_bytes == 0 {
            return Err(anyhow!("email_content_expired"));
        }
        Ok(())
    }

    fn read_raw(&self, mail: &Mail) -> Result<Vec<u8>> {
        Self::check_content(mail)?;
        let path = self.raw_root.join(format!("{}.eml", mail.id));
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| anyhow!("email_content_unavailable"))?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > content::MAX_MESSAGE_BYTES as u64
        {
            return Err(anyhow!("email_content_unavailable"));
        }
        fs::read(path).map_err(|_| anyhow!("email_content_unavailable"))
    }

    pub fn attachment_metadata(&self, mail: &Mail) -> Result<Vec<content::Attachment>> {
        Self::check_content(mail)?;
        let metadata: Option<String> = self.conn.lock().query_row(
            "SELECT attachments FROM mail WHERE id=?",
            [&mail.id],
            |row| row.get(0),
        )?;
        if let Some(metadata) = metadata {
            return Ok(serde_json::from_str(&metadata)?);
        }
        // Schema 1 records acquire metadata once, without changing their saved
        // normalized body or requiring an eager scan of the raw mail archive.
        let attachments = content::attachments(&self.read_raw(mail)?)?;
        let metadata = serde_json::to_string(&attachments)?;
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE mail SET attachments=?,body_bytes=body_bytes+? WHERE id=? AND attachments IS NULL AND body IS NOT NULL AND raw_bytes>0",
            params![metadata, metadata.len() as i64, mail.id],
        )?;
        if changed > 0 {
            tx.execute(
                "UPDATE stats SET value=value+? WHERE key='content_bytes'",
                [metadata.len() as i64],
            )?;
        }
        tx.commit()?;
        Ok(attachments)
    }

    pub fn read_image(&self, mail: &Mail) -> Result<content::Content> {
        let mut content = content::image(&self.read_raw(mail)?)?;
        content.body = mail.body.clone().unwrap_or_default();
        Ok(content)
    }

    pub fn read_attachment(&self, mail: &Mail, id: &str) -> Result<content::Attachment> {
        content::attachment(&self.read_raw(mail)?, id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_send(
        &self,
        account: &str,
        raw: &[u8],
        body: &str,
        from: &str,
        to: &[String],
        cc: &[String],
        subject: &str,
    ) -> Result<String> {
        let id = format!("mail_{}", Uuid::new_v4().simple());
        let parsed = content::parse(raw)?;
        let attachments = serde_json::to_string(&parsed.attachments)?;
        let body_bytes = body.len() + attachments.len();
        self.write_raw(&id, raw)?;
        let result = (|| -> Result<()> {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            tx.execute("INSERT INTO mail(id,account,inbox,sender,recipients,cc,subject,received_at,unread,status,snippet,body,format,message_id,refs,raw_bytes,body_bytes,attachments)
            VALUES(?,?,0,?,?,?,?,?,0,'pending',?,?,'plain',?,?,?,?,?)", params![id,account,from,to.join(", "),cc.join(", "),subject,Utc::now().timestamp_millis(),snippet(body),body,parsed.message_id,parsed.references,raw.len() as i64,body_bytes as i64,attachments])?;
            tx.execute(
                "UPDATE stats SET value=value+? WHERE key='content_bytes'",
                [(raw.len() + body_bytes) as i64],
            )?;
            tx.commit()?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(self.raw_root.join(format!("{id}.eml")));
            return Err(error);
        }
        Ok(id)
    }

    pub fn sent_status(&self, id: &str, status: &str) -> Result<()> {
        self.conn
            .lock()
            .execute("UPDATE mail SET status=? WHERE id=?", params![status, id])?;
        Ok(())
    }

    pub fn apply_operation(&self, id: &str, action: &str) -> Result<()> {
        let conn = self.conn.lock();
        match action {
            "trash" => conn.execute("UPDATE mail SET inbox=0 WHERE id=?", [id])?,
            "markRead" => conn.execute("UPDATE mail SET unread=0 WHERE id=?", [id])?,
            "markUnread" => conn.execute("UPDATE mail SET unread=1 WHERE id=?", [id])?,
            "flag" => conn.execute("UPDATE mail SET flagged=1 WHERE id=?", [id])?,
            "unflag" => conn.execute("UPDATE mail SET flagged=0 WHERE id=?", [id])?,
            _ => return Err(anyhow!("email_action_invalid")),
        };
        Ok(())
    }

    pub fn pending_events(&self) -> Result<Vec<(String, String)>> {
        let conn = self.conn.lock();
        let mut statement =
            conn.prepare("SELECT id,message FROM outbox ORDER BY rowid LIMIT 100")?;
        let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn acknowledge_event(&self, id: &str) -> Result<()> {
        self.conn
            .lock()
            .execute("DELETE FROM outbox WHERE id=?", [id])?;
        Ok(())
    }

    pub fn cleanup(&self, retention_days: u64, max_bytes: u64) -> Result<()> {
        if self.orphan_cleanup_needed.load(Ordering::Acquire) {
            self.remove_orphans()?;
            self.orphan_cleanup_needed.store(false, Ordering::Release);
        }
        let cutoff = Utc::now().timestamp_millis().saturating_sub(
            retention_days
                .saturating_mul(86_400_000)
                .min(i64::MAX as u64) as i64,
        );
        let mut conn = self.conn.lock();
        let mut total: u64 = conn.query_row(
            "SELECT value FROM stats WHERE key='content_bytes'",
            [],
            |row| row.get(0),
        )?;
        let tx = conn.transaction()?;
        let mut statement=tx.prepare("SELECT id,received_at,raw_bytes+body_bytes FROM mail WHERE raw_bytes>0 AND status!='pending' ORDER BY received_at,id")?;
        let mut rows = statement.query([])?;
        let mut expired = Vec::new();
        while let Some(row) = rows.next()? {
            let received_at: i64 = row.get(1)?;
            if received_at >= cutoff && total <= max_bytes {
                break;
            }
            let id: String = row.get(0)?;
            let bytes: u64 = row.get(2)?;
            total = total.saturating_sub(bytes);
            expired.push(id);
        }
        drop(rows);
        drop(statement);
        for id in &expired {
            // Commit expiry before deleting files. Reopen (or the next cleanup
            // after an unlink failure) removes orphans without losing mail IDs.
            tx.execute(
                "UPDATE mail SET body=NULL,raw_bytes=0,body_bytes=0,attachments=NULL WHERE id=?",
                [id],
            )?;
        }
        tx.execute(
            "UPDATE stats SET value=? WHERE key='content_bytes'",
            [total],
        )?;
        tx.commit()?;
        for id in &expired {
            let path = self.raw_root.join(format!("{id}.eml"));
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    self.orphan_cleanup_needed.store(true, Ordering::Release);
                    return Err(error.into());
                }
            }
        }
        if !expired.is_empty() {
            conn.execute_batch("PRAGMA incremental_vacuum")?;
        }
        Ok(())
    }

    fn write_raw(&self, id: &str, raw: &[u8]) -> Result<()> {
        let target = self.raw_root.join(format!("{id}.eml"));
        let temporary = self
            .raw_root
            .join(format!(".pending-{}", Uuid::new_v4().simple()));
        let result = (|| -> Result<()> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(raw)?;
            file.sync_all()?;
            fs::hard_link(&temporary, &target)?;
            fs::remove_file(&temporary)?;
            fs::File::open(&self.raw_root)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn row_mail(row: &rusqlite::Row<'_>) -> rusqlite::Result<Mail> {
    Ok(Mail {
        id: row.get(0)?,
        account: row.get(1)?,
        validity: row.get(2)?,
        uid: row.get(3)?,
        inbox: row.get(4)?,
        from: row.get(5)?,
        to: row.get(6)?,
        cc: row.get(7)?,
        subject: row.get(8)?,
        received_at: row.get(9)?,
        unread: row.get(10)?,
        status: row.get(11)?,
        snippet: row.get(12)?,
        body: row.get(13)?,
        format: row.get(14)?,
        message_id: row.get(15)?,
        references: row.get(16)?,
        raw_bytes: row.get(17)?,
    })
}

pub(super) fn timestamp(value: i64) -> String {
    DateTime::from_timestamp_millis(value)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        .to_rfc3339()
}

fn snippet(body: &str) -> String {
    bounded(&body.split_whitespace().collect::<Vec<_>>().join(" "), 240)
}
fn bounded(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn private_directory(path: &Path) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(anyhow!("email_private_directory_invalid"));
        }
    }
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
