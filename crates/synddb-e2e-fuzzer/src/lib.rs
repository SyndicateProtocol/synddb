//! End-to-end fuzzer for `SyndDB` replication pipeline
//!
//! This crate tests the full `SyndDB` pipeline:
//! - Client: SQL execution, changeset capture
//! - Sequencer: Sequence assignment, COSE signing, compression
//! - Validator: Signature verification, changeset application, audit trail
//!
//! Unlike `synddb-fuzzer` which only tests `SQLite` changeset roundtrip,
//! this fuzzer exercises cross-component invariants.

pub mod faults;
pub mod harness;
pub mod invariants;
pub mod property_tests;
pub mod replay;
pub mod scenarios;

pub use harness::E2EHarness;
pub use invariants::{InvariantChecker, InvariantViolation};
pub use replay::{run_seeded_test, scenario_from_seed};
pub use scenarios::{E2EAction, E2EScenario};
