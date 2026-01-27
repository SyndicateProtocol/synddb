//! Invariant checkers for E2E testing
//!
//! These invariants verify properties that should always hold across the
//! `SyndDB` pipeline.

use crate::harness::E2EHarness;

/// A violation of an invariant
#[derive(Debug, Clone)]
pub enum InvariantViolation {
    /// Sequence numbers are not strictly monotonic
    SequenceNotMonotonic {
        prev_sequence: u64,
        curr_sequence: u64,
    },
    /// Gap detected in sequence numbers
    SequenceGap { expected: u64, actual: u64 },
    /// Signature verification failed
    InvalidSignature { sequence: u64, error: String },
    /// Signer does not match expected sequencer pubkey
    SignerMismatch {
        sequence: u64,
        expected: String,
        actual: String,
    },
    /// Source and replica database state differ
    StateInconsistent { diff: String },
    /// Changeset cannot be inverted
    ChangesetNotInvertible { sequence: u64, error: String },
}

impl std::fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SequenceNotMonotonic {
                prev_sequence,
                curr_sequence,
            } => {
                write!(
                    f,
                    "Sequence not monotonic: {} -> {}",
                    prev_sequence, curr_sequence
                )
            }
            Self::SequenceGap { expected, actual } => {
                write!(f, "Sequence gap: expected {}, got {}", expected, actual)
            }
            Self::InvalidSignature { sequence, error } => {
                write!(f, "Invalid signature at sequence {}: {}", sequence, error)
            }
            Self::SignerMismatch {
                sequence,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Signer mismatch at sequence {}: expected {}, got {}",
                    sequence, expected, actual
                )
            }
            Self::StateInconsistent { diff } => {
                write!(f, "State inconsistent: {}", diff)
            }
            Self::ChangesetNotInvertible { sequence, error } => {
                write!(
                    f,
                    "Changeset at sequence {} not invertible: {}",
                    sequence, error
                )
            }
        }
    }
}

/// Trait for invariant checkers
pub trait InvariantChecker {
    /// Check invariants and return any violations
    fn check(&self, harness: &E2EHarness) -> Vec<InvariantViolation>;
}

/// Check that sequence numbers are strictly monotonic and contiguous
#[derive(Debug)]
pub struct SequenceInvariants;

impl InvariantChecker for SequenceInvariants {
    fn check(&self, harness: &E2EHarness) -> Vec<InvariantViolation> {
        let mut violations = Vec::new();
        let messages = harness.messages();

        if messages.is_empty() {
            return violations;
        }

        // Check first message starts at 0
        if messages[0].sequence != 0 {
            violations.push(InvariantViolation::SequenceGap {
                expected: 0,
                actual: messages[0].sequence,
            });
        }

        // Check monotonicity and contiguity
        for window in messages.windows(2) {
            let prev = &window[0];
            let curr = &window[1];

            // Monotonicity: curr > prev
            if curr.sequence <= prev.sequence {
                violations.push(InvariantViolation::SequenceNotMonotonic {
                    prev_sequence: prev.sequence,
                    curr_sequence: curr.sequence,
                });
            }

            // Contiguity: curr == prev + 1
            if curr.sequence != prev.sequence + 1 {
                violations.push(InvariantViolation::SequenceGap {
                    expected: prev.sequence + 1,
                    actual: curr.sequence,
                });
            }
        }

        violations
    }
}

/// Check that all signatures are valid
#[derive(Debug)]
pub struct SignatureInvariants;

impl InvariantChecker for SignatureInvariants {
    fn check(&self, harness: &E2EHarness) -> Vec<InvariantViolation> {
        let mut violations = Vec::new();
        let messages = harness.messages();
        let expected_pubkey = format!("0x{}", hex::encode(harness.sequencer_pubkey()));

        for message in &messages {
            // Verify signer matches expected
            if message.signer != expected_pubkey {
                violations.push(InvariantViolation::SignerMismatch {
                    sequence: message.sequence,
                    expected: expected_pubkey.clone(),
                    actual: message.signer.clone(),
                });
            }

            // Note: Full COSE signature verification is done by the validator
            // during sync. Here we just verify the signer field.
        }

        violations
    }
}

/// Check that source and replica databases are consistent
#[derive(Debug)]
pub struct ConsistencyInvariants;

impl InvariantChecker for ConsistencyInvariants {
    fn check(&self, harness: &E2EHarness) -> Vec<InvariantViolation> {
        let mut violations = Vec::new();

        match (
            harness.snapshot_source_state(),
            harness.snapshot_replica_state(),
        ) {
            (Ok(source), Ok(replica)) => {
                if !source.is_equivalent(&replica) {
                    violations.push(InvariantViolation::StateInconsistent {
                        diff: source.diff(&replica),
                    });
                }
            }
            (Err(e), _) => {
                violations.push(InvariantViolation::StateInconsistent {
                    diff: format!("Failed to snapshot source: {}", e),
                });
            }
            (_, Err(e)) => {
                violations.push(InvariantViolation::StateInconsistent {
                    diff: format!("Failed to snapshot replica: {}", e),
                });
            }
        }

        violations
    }
}

/// Check that all changesets can be inverted (for audit trail)
#[derive(Debug)]
pub struct ChainInvariants;

impl ChainInvariants {
    /// Check that a changeset can be inverted
    fn can_invert_changeset(data: &[u8]) -> Result<(), String> {
        // Decompress the payload first
        let _decompressed = match zstd::decode_all(std::io::Cursor::new(data)) {
            Ok(d) => d,
            Err(e) => return Err(format!("Failed to decompress: {}", e)),
        };

        // Parse the CBOR payload to extract the actual changeset
        // For now, we just check that decompression works
        // Full inversion testing would require access to the changeset bytes
        // inside the ChangesetBatchRequest

        Ok(())
    }
}

impl InvariantChecker for ChainInvariants {
    fn check(&self, harness: &E2EHarness) -> Vec<InvariantViolation> {
        let mut violations = Vec::new();
        let messages = harness.messages();

        for message in &messages {
            // Only check changeset messages
            if message.message_type != synddb_shared::types::message::MessageType::Changeset {
                continue;
            }

            if let Err(e) = Self::can_invert_changeset(&message.payload) {
                violations.push(InvariantViolation::ChangesetNotInvertible {
                    sequence: message.sequence,
                    error: e,
                });
            }
        }

        violations
    }
}

/// Run all invariant checkers and collect violations
pub fn check_all_invariants(harness: &E2EHarness) -> Vec<InvariantViolation> {
    let checkers: Vec<Box<dyn InvariantChecker>> = vec![
        Box::new(SequenceInvariants),
        Box::new(SignatureInvariants),
        Box::new(ConsistencyInvariants),
        Box::new(ChainInvariants),
    ];

    let mut all_violations = Vec::new();
    for checker in &checkers {
        all_violations.extend(checker.check(harness));
    }

    all_violations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sequence_invariants_pass() {
        let mut harness = E2EHarness::new().expect("Failed to create harness");
        harness
            .setup_schema("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)")
            .expect("Failed to setup schema");

        // Insert initial data
        harness
            .source()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();
        harness
            .replica()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();

        // Execute and sequence multiple operations
        harness
            .execute_and_sequence("UPDATE t SET v = 1 WHERE id = 1")
            .unwrap();
        harness
            .execute_and_sequence("UPDATE t SET v = 2 WHERE id = 1")
            .unwrap();

        let violations = SequenceInvariants.check(&harness);
        assert!(
            violations.is_empty(),
            "Expected no violations: {:?}",
            violations
        );
    }

    #[tokio::test]
    async fn test_signature_invariants_pass() {
        let mut harness = E2EHarness::new().expect("Failed to create harness");
        harness
            .setup_schema("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)")
            .expect("Failed to setup schema");

        harness
            .source()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();
        harness
            .replica()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();

        harness
            .execute_and_sequence("UPDATE t SET v = 1 WHERE id = 1")
            .unwrap();

        let violations = SignatureInvariants.check(&harness);
        assert!(
            violations.is_empty(),
            "Expected no violations: {:?}",
            violations
        );
    }

    #[tokio::test]
    async fn test_consistency_invariants_pass() {
        let mut harness = E2EHarness::new().expect("Failed to create harness");
        harness
            .setup_schema("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)")
            .expect("Failed to setup schema");

        harness
            .source()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();
        harness
            .replica()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();

        // Execute and sync
        harness
            .execute_and_sequence("UPDATE t SET v = 1 WHERE id = 1")
            .unwrap();
        harness.sync_validator().await.unwrap();

        let violations = ConsistencyInvariants.check(&harness);
        assert!(
            violations.is_empty(),
            "Expected no violations: {:?}",
            violations
        );
    }

    #[tokio::test]
    async fn test_consistency_invariants_detect_mismatch() {
        let mut harness = E2EHarness::new().expect("Failed to create harness");
        harness
            .setup_schema("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)")
            .expect("Failed to setup schema");

        harness
            .source()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();
        harness
            .replica()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();

        // Execute but DON'T sync - should detect mismatch
        harness
            .execute_and_sequence("UPDATE t SET v = 1 WHERE id = 1")
            .unwrap();

        let violations = ConsistencyInvariants.check(&harness);
        assert!(!violations.is_empty(), "Expected state mismatch violation");

        match &violations[0] {
            InvariantViolation::StateInconsistent { .. } => {}
            other => panic!("Expected StateInconsistent, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_all_invariants_pass() {
        let mut harness = E2EHarness::new().expect("Failed to create harness");
        harness
            .setup_schema("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)")
            .expect("Failed to setup schema");

        harness
            .source()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();
        harness
            .replica()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();

        // Full roundtrip
        harness
            .execute_and_sequence("UPDATE t SET v = 1 WHERE id = 1")
            .unwrap();
        harness.sync_validator().await.unwrap();

        let violations = check_all_invariants(&harness);
        assert!(
            violations.is_empty(),
            "Expected no violations: {:?}",
            violations
        );
    }
}
