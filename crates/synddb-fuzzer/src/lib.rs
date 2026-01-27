//! `SQLite` Fuzzer for `SyndDB`
//!
//! This crate provides property-based and grammar-based testing for `SyndDB`'s
//! `SQLite` replication system. It verifies that:
//!
//! 1. All database changes are captured as changesets
//! 2. Changesets can be replayed to produce identical database state
//! 3. Edge cases are handled correctly (NULL, empty strings, large values, etc.)
//!
//! # Property-Based Testing
//!
//! Uses `proptest` to generate random sequences of SQL operations and verify
//! that `SyndDB` handles them correctly.
//!
//! # Grammar-Based Testing
//!
//! Generates valid SQL statements using a grammar-based approach to test
//! complex query patterns and edge cases.

pub mod generators;
pub mod grammar;
pub mod operations;
pub mod property_tests;
pub mod replay;

pub use generators::*;
pub use grammar::*;
pub use operations::*;
