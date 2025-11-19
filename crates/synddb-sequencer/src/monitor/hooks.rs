//! SQLite commit and update hooks for changeset extraction

use anyhow::Result;

/// Commit hook callback - fires after successful COMMIT
pub struct CommitHook {
    // TODO: Add callback state
}

impl Default for CommitHook {
    fn default() -> Self {
        Self::new()
    }
}

impl CommitHook {
    pub fn new() -> Self {
        Self {}
    }

    /// Called by SQLite after a transaction commits
    pub fn on_commit(&mut self) -> Result<()> {
        // TODO: Extract changeset from session
        // TODO: Send to changeset channel
        Ok(())
    }
}

/// Update hook callback - fires on INSERT/UPDATE/DELETE
pub struct UpdateHook {
    // TODO: Add callback state
}

impl Default for UpdateHook {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateHook {
    pub fn new() -> Self {
        Self {}
    }

    /// Called by SQLite on data modification
    pub fn on_update(&mut self, _table: &str, _rowid: i64) -> Result<()> {
        // TODO: Track modifications
        // TODO: Detect sqlite_schema changes for DDL tracking
        Ok(())
    }
}
