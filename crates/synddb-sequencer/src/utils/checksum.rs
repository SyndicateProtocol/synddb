//! Data integrity checksums

use sha2::{Digest, Sha256};

/// Compute SHA256 hash of data
pub fn compute_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    hex::encode(result)
}

/// Verify hash matches data
pub fn verify_hash(data: &[u8], expected_hash: &str) -> bool {
    let computed = compute_hash(data);
    computed == expected_hash
}
