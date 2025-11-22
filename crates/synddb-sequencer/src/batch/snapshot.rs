//! Full database snapshot creation

use anyhow::Result;
use std::path::Path;

#[derive(Debug)]
pub struct SnapshotCreator {
    // TODO: Add state
}

impl Default for SnapshotCreator {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotCreator {
    pub const fn new() -> Self {
        Self {}
    }

    /// Create a full snapshot of the database
    pub async fn create_snapshot(&self, _db_path: &Path) -> Result<Vec<u8>> {
        // TODO: Use SQLite backup API to create snapshot
        // See https://www.sqlite.org/backup.html
        Ok(vec![])
    }

    /// Include PRAGMA settings in snapshot metadata
    pub async fn capture_pragmas(&self, _db_path: &Path) -> Result<PragmaSettings> {
        // TODO: Query important PRAGMA settings
        Ok(PragmaSettings::default())
    }
}

#[derive(Debug, Clone, Default)]
pub struct PragmaSettings {
    pub foreign_keys: bool,
    pub recursive_triggers: bool,
    pub secure_delete: bool,
    pub encoding: String,
    pub user_version: i32,
}
