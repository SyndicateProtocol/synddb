//! DDL crash recovery via marker files
//!
//! When DDL is executed via `execute_ddl()`, we write a marker file before
//! execution. On successful snapshot creation, the marker is cleared. On startup,
//! if this marker exists, we force an immediate snapshot to recover from a crash
//! that occurred during DDL execution.
//!
//! This provides crash recovery for the case where:
//! 1. `execute_ddl()` writes marker
//! 2. DDL executes but app crashes before snapshot completes
//! 3. On restart, marker triggers recovery snapshot

// Allow pub visibility in this private module to avoid conflicting clippy lints
#![allow(unreachable_pub)]

use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::PathBuf,
};
use tracing::{debug, info, warn};

/// Get the marker file path for a given database path
fn marker_path(db_path: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    db_path.hash(&mut hasher);
    let hash = hasher.finish();

    std::env::temp_dir().join(format!("synddb_ddl_pending_{:x}.marker", hash))
}

/// Write the DDL pending marker file
///
/// Called when direct DDL is detected (not via `execute_ddl()`).
/// This marker will be checked on next startup to force a snapshot.
pub fn write_marker(db_path: &str) {
    let path = marker_path(db_path);
    match fs::write(&path, "schema_change_pending") {
        Ok(()) => {
            debug!("DDL recovery marker written: {:?}", path);
        }
        Err(e) => {
            // Don't fail on marker write - it's best effort
            warn!("Failed to write DDL recovery marker: {}", e);
        }
    }
}

/// Check if a DDL pending marker exists
///
/// Returns true if there was a pending schema change from a previous run
/// that wasn't properly published (likely due to crash).
pub fn check_marker(db_path: &str) -> bool {
    let path = marker_path(db_path);
    path.exists()
}

/// Clear the DDL pending marker
///
/// Called after a snapshot is successfully published, indicating
/// the schema change has been persisted to the sequencer.
pub fn clear_marker(db_path: &str) {
    let path = marker_path(db_path);
    if path.exists() {
        match fs::remove_file(&path) {
            Ok(()) => {
                info!("DDL recovery marker cleared: {:?}", path);
            }
            Err(e) => {
                warn!("Failed to clear DDL recovery marker: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_marker_lifecycle() {
        let db_path = "/tmp/test_marker_lifecycle.db";

        // Initially no marker
        clear_marker(db_path); // Clean up from previous runs
        assert!(!check_marker(db_path));

        // Write marker
        write_marker(db_path);
        assert!(check_marker(db_path));

        // Clear marker
        clear_marker(db_path);
        assert!(!check_marker(db_path));
    }

    #[test]
    fn test_marker_path_uniqueness() {
        let path1 = marker_path("/path/to/db1.db");
        let path2 = marker_path("/path/to/db2.db");
        let path3 = marker_path("/path/to/db1.db");

        assert_ne!(path1, path2);
        assert_eq!(path1, path3);
    }
}
