use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(crate) fn random_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

pub(crate) fn random_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub(crate) fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn constant_time_equal(a: &str, b: &str) -> bool {
    let max = a.len().max(b.len());
    let mut diff = a.len() ^ b.len();
    for index in 0..max {
        diff |= a.as_bytes().get(index).copied().unwrap_or(0) as usize
            ^ b.as_bytes().get(index).copied().unwrap_or(0) as usize;
    }
    diff == 0
}
