//! Fault injection for E2E testing
//!
//! Provides mechanisms to inject faults into the `SyndDB` pipeline to test
//! error handling and recovery.

use crate::harness::E2EHarness;
use synddb_validator::sync::fetcher::StorageFetcher;

/// Types of payload corruption for testing
#[derive(Debug, Clone)]
pub enum CorruptionType {
    /// Flip random bits in the payload
    BitFlip,
    /// Truncate the payload
    Truncate { keep_bytes: usize },
    /// Corrupt the CBOR structure
    CorruptCbor,
    /// Zero out the payload
    ZeroOut,
}

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
    /// Corrupt a message's payload
    CorruptPayload {
        sequence: u64,
        corruption_type: CorruptionType,
    },
    /// Swap the order of two messages in storage
    SwapMessages { sequence1: u64, sequence2: u64 },
    /// Duplicate a message (store same content at different sequence)
    DuplicateMessage {
        source_sequence: u64,
        target_sequence: u64,
    },
    /// Replace signer with a different (invalid) one
    WrongSigner { sequence: u64, fake_signer: String },
    /// Create a gap in sequence numbers
    CreateSequenceGap { gap_at: u64 },
}

/// Run an async operation, handling both sync and async contexts
fn run_async<F, T>(f: F) -> T
where
    F: FnOnce() -> T + Send,
    T: Send,
{
    // Check if we're already in an async context
    if tokio::runtime::Handle::try_current().is_ok() {
        // We're in an async context, spawn a thread to avoid nested runtime
        std::thread::scope(|s| s.spawn(f).join().expect("Thread panicked"))
    } else {
        // Not in async context, just run directly
        f()
    }
}

/// Inject a fault into the harness
///
/// Note: Some faults require async operations and handle nested runtime contexts.
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
        Fault::CorruptPayload {
            sequence,
            corruption_type,
        } => {
            let seq = *sequence;
            let corruption = corruption_type.clone();
            let fetcher = harness.fetcher();

            run_async(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create runtime");

                if let Ok(Some(mut msg)) = rt.block_on(fetcher.get(seq)) {
                    msg.payload = corrupt_payload(&msg.payload, &corruption);
                    fetcher.clear();
                    fetcher.add_message(msg);
                }
            });
        }
        Fault::SwapMessages {
            sequence1,
            sequence2,
        } => {
            let seq1 = *sequence1;
            let seq2 = *sequence2;
            let fetcher = harness.fetcher();

            run_async(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create runtime");

                let msg1 = rt.block_on(fetcher.get(seq1));
                let msg2 = rt.block_on(fetcher.get(seq2));

                if let (Ok(Some(mut m1)), Ok(Some(mut m2))) = (msg1, msg2) {
                    std::mem::swap(&mut m1.sequence, &mut m2.sequence);
                    fetcher.clear();
                    fetcher.add_message(m1);
                    fetcher.add_message(m2);
                }
            });
        }
        Fault::DuplicateMessage {
            source_sequence,
            target_sequence,
        } => {
            let src_seq = *source_sequence;
            let tgt_seq = *target_sequence;
            let fetcher = harness.fetcher();

            run_async(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create runtime");

                if let Ok(Some(mut msg)) = rt.block_on(fetcher.get(src_seq)) {
                    msg.sequence = tgt_seq;
                    fetcher.add_message(msg);
                }
            });
        }
        Fault::WrongSigner {
            sequence,
            fake_signer,
        } => {
            let seq = *sequence;
            let signer = fake_signer.clone();
            let fetcher = harness.fetcher();

            run_async(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create runtime");

                if let Ok(Some(mut msg)) = rt.block_on(fetcher.get(seq)) {
                    msg.signer = signer;
                    fetcher.clear();
                    fetcher.add_message(msg);
                }
            });
        }
        Fault::CreateSequenceGap { gap_at } => {
            let gap = *gap_at;
            let fetcher = harness.fetcher();

            run_async(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create runtime");

                // Get all messages and shift sequences after the gap
                let mut messages = Vec::new();
                let mut seq = 0;
                while let Ok(Some(msg)) = rt.block_on(fetcher.get(seq)) {
                    messages.push(msg);
                    seq += 1;
                }

                fetcher.clear();
                for mut msg in messages {
                    if msg.sequence >= gap {
                        msg.sequence += 1;
                    }
                    fetcher.add_message(msg);
                }
            });
        }
    }
}

/// Corrupt a payload according to the corruption type
fn corrupt_payload(payload: &[u8], corruption_type: &CorruptionType) -> Vec<u8> {
    match corruption_type {
        CorruptionType::BitFlip => {
            let mut corrupted = payload.to_vec();
            if !corrupted.is_empty() {
                // Flip bits in first byte
                corrupted[0] ^= 0xFF;
            }
            corrupted
        }
        CorruptionType::Truncate { keep_bytes } => {
            payload[..*keep_bytes.min(&payload.len())].to_vec()
        }
        CorruptionType::CorruptCbor => {
            // Insert invalid CBOR marker at the beginning
            let mut corrupted = vec![0xFF, 0xFF]; // Invalid CBOR
            corrupted.extend_from_slice(payload);
            corrupted
        }
        CorruptionType::ZeroOut => {
            vec![0u8; payload.len()]
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
    use synddb_shared::types::message::{MessageType, SignedMessage};
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
        let mut message = SignedMessage {
            sequence: 0,
            timestamp: 0,
            message_type: MessageType::Changeset,
            payload: vec![],
            message_hash: String::new(),
            signature: String::new(),
            signer: String::new(),
            cose_protected_header: vec![],
        };

        tamper_sequence(&mut message, 999);
        assert_eq!(message.sequence, 999);
    }

    #[test]
    fn test_corrupt_payload_bitflip() {
        let payload = vec![0x00, 0x01, 0x02, 0x03];
        let corrupted = corrupt_payload(&payload, &CorruptionType::BitFlip);
        assert_eq!(corrupted[0], 0xFF); // First byte flipped
        assert_eq!(corrupted[1..], payload[1..]); // Rest unchanged
    }

    #[test]
    fn test_corrupt_payload_truncate() {
        let payload = vec![0x00, 0x01, 0x02, 0x03, 0x04, 0x05];
        let corrupted = corrupt_payload(&payload, &CorruptionType::Truncate { keep_bytes: 3 });
        assert_eq!(corrupted.len(), 3);
        assert_eq!(corrupted, &payload[..3]);
    }

    #[test]
    fn test_corrupt_payload_zero_out() {
        let payload = vec![0x01, 0x02, 0x03];
        let corrupted = corrupt_payload(&payload, &CorruptionType::ZeroOut);
        assert_eq!(corrupted.len(), payload.len());
        assert!(corrupted.iter().all(|&b| b == 0));
    }

    #[tokio::test]
    async fn test_wrong_signer_injection() {
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

        // Inject wrong signer
        inject_fault(
            &harness,
            &Fault::WrongSigner {
                sequence: 0,
                fake_signer: "0xfake_signer".to_string(),
            },
        );

        // Fetch and verify signer was changed
        let msg = harness.fetcher().get(0).await.unwrap().unwrap();
        assert_eq!(msg.signer, "0xfake_signer");
    }

    #[tokio::test]
    async fn test_corrupt_payload_injection() {
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

        // Get original payload
        let original_msg = harness.fetcher().get(0).await.unwrap().unwrap();
        let original_first_byte = original_msg.payload[0];

        // Inject corruption
        inject_fault(
            &harness,
            &Fault::CorruptPayload {
                sequence: 0,
                corruption_type: CorruptionType::BitFlip,
            },
        );

        // Fetch and verify payload was corrupted
        let msg = harness.fetcher().get(0).await.unwrap().unwrap();
        assert_ne!(msg.payload[0], original_first_byte);
    }
}
