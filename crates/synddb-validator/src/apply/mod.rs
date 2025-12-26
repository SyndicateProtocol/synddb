//! Changeset application module
//!
//! Applies `SQLite` changesets from sequenced messages to reconstruct state.

pub mod applier;
pub mod audit;
