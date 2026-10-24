//! Best-effort metadata only; never stores item names, credentials, or values.
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::PathBuf,
};
#[derive(Default, Serialize, Deserialize)]
struct Record {
    weight: f64,
    used_at: i64,
}
pub(super) struct Usage {
    path: PathBuf,
    records: parking_lot::Mutex<BTreeMap<String, Record>>,
}
impl Usage {
    pub(super) fn open(path: PathBuf) -> Self {
        let records = std::fs::symlink_metadata(&path)
            .ok()
            .filter(|m| m.is_file() && !m.file_type().is_symlink() && m.len() <= 1024 * 1024)
            .and_then(|_| std::fs::read(&path).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            path,
            records: parking_lot::Mutex::new(records),
        }
    }
    pub(super) fn rank(&self, id: &str) -> (f64, i64) {
        self.records
            .lock()
            .get(id)
            .map(|r| (decay(r), r.used_at))
            .unwrap_or((0.0, 0))
    }
    pub(super) fn used(&self, id: &str) {
        let mut records = self.records.lock();
        let entry = records.entry(id.to_owned()).or_default();
        entry.weight = decay(entry) + 1.0;
        entry.used_at = Utc::now().timestamp();
        self.save(&records);
    }
    pub(super) fn remove(&self, id: &str) {
        let mut records = self.records.lock();
        records.remove(id);
        self.save(&records);
    }
    pub(super) fn prune(&self, ids: &BTreeSet<String>) {
        let mut records = self.records.lock();
        records.retain(|id, _| ids.contains(id));
        self.save(&records);
    }
    fn save(&self, records: &BTreeMap<String, Record>) {
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
        let result = (|| -> std::io::Result<()> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            serde_json::to_writer(&mut file, records)?;
            file.flush()?;
            file.sync_all()?;
            std::fs::rename(&temporary, &self.path)?;
            if let Some(parent) = self.path.parent() {
                std::fs::File::open(parent)?.sync_all()?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
    }
}
fn decay(record: &Record) -> f64 {
    if !record.weight.is_finite() || record.weight < 0.0 {
        return 0.0;
    }
    record.weight
        * 2f64.powf(
            -(Utc::now().timestamp().saturating_sub(record.used_at).max(0) as f64)
                / (30.0 * 86400.0),
        )
}
