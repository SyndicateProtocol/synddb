//! Fault injection for E2E testing
//!
//! Provides mechanisms to inject faults into the `SyndDB` pipeline to test
//! error handling and recovery.

use crate::harness::E2EHarness;

/// Types of faults that can be injected
#[derive(Debug, Clone)]
pub enum Fault {
    /// Make a specific sequence fail to fetch
    MessageFetchFailure {
        sequence: u64,
        error_message: String,
    },
    /// Make the latest sequence query fail
    LatestSequenceFailure { error_message: String },
    /// Clear all injected failures
    ClearFailures,
}

/// Inject a fault into the harness
pub fn inject_fault(harness: &E2EHarness, fault: &Fault) {
    match fault {
        Fault::MessageFetchFailure {
            sequence,
            error_message,
        } => {
            harness.fetcher().set_fail_get(*sequence, error_message);
        }
        Fault::LatestSequenceFailure { error_message } => {
            harness.fetcher().set_fail_latest(error_message);
        }
        Fault::ClearFailures => {
            harness.fetcher().clear_failures();
        }
    }
}

/// Tamper with a signed message's signature
pub fn tamper_signature(signature: &str) -> String {
    // Parse the signature and flip some bits
    if let Some(hex_str) = signature.strip_prefix("0x") {
        if let Ok(mut bytes) = hex::decode(hex_str) {
            if !bytes.is_empty() {
                let last_idx = bytes.len() - 1;
                bytes[last_idx] ^= 0xFF; // Flip last byte
            }
            return format!("0x{}", hex::encode(bytes));
        }
    }
    signature.to_string()
}

/// Tamper with a signed message's sequence number
pub const fn tamper_sequence(
    message: &mut synddb_shared::types::message::SignedMessage,
    new_sequence: u64,
) {
    message.sequence = new_sequence;
}

#[cfg(test)]
mod tests {
    use super::*;
    use synddb_validator::sync::fetcher::StorageFetcher;

    #[tokio::test]
    async fn test_message_fetch_failure() {
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

        // Sequence a message
        harness
            .execute_and_sequence("UPDATE t SET v = 1 WHERE id = 1")
            .unwrap();

        // Inject failure for sequence 0
        inject_fault(
            &harness,
            &Fault::MessageFetchFailure {
                sequence: 0,
                error_message: "injected failure".to_string(),
            },
        );

        // Sync should fail
        let result = harness.fetcher().get(0).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("injected failure"));

        // Clear failures
        inject_fault(&harness, &Fault::ClearFailures);

        // Should work now
        let result = harness.fetcher().get(0).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_latest_sequence_failure() {
        let harness = E2EHarness::new().expect("Failed to create harness");

        // Inject failure
        inject_fault(
            &harness,
            &Fault::LatestSequenceFailure {
                error_message: "network error".to_string(),
            },
        );

        // Query should fail
        let result = harness.fetcher().get_latest_sequence().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("network error"));
    }

    #[test]
    fn test_tamper_signature() {
        let original = "0x0123456789abcdef";
        let tampered = tamper_signature(original);
        assert_ne!(original, tampered);
        assert!(tampered.starts_with("0x"));
    }

    #[test]
    fn test_tamper_sequence() {
        let mut message = synddb_shared::types::message::SignedMessage {
            sequence: 0,
            timestamp: 0,
            message_type: synddb_shared::types::message::MessageType::Changeset,
            payload: vec![],
            message_hash: String::new(),
            signature: String::new(),
            signer: String::new(),
            cose_protected_header: vec![],
        };

        tamper_sequence(&mut message, 999);
        assert_eq!(message.sequence, 999);
    }
}
