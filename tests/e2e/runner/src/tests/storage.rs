//! Storage layer fetch tests

use anyhow::ensure;
use synddb_shared::types::message::MessageType;

use crate::{
    result::{TestCase, TestCaseResult},
    runner::TestRunner,
};

impl TestRunner {
    /// Test: Can fetch messages from the storage layer
    pub(crate) async fn test_storage_fetch(&self) -> TestCaseResult {
        TestCase::new("storage_fetch", "Storage message fetch works")
            .run(|| async {
                // Get the latest sequence from storage
                let latest = self.sequencer.storage_latest().await?;
                ensure!(latest.sequence.is_some(), "No messages in storage layer");

                // Fetch message at sequence 0 (first message)
                let message = self.sequencer.fetch_storage_message(0).await?;

                // Verify basic message structure
                ensure!(
                    message.sequence == 0,
                    "Expected sequence 0, got {}",
                    message.sequence
                );
                // First message can be either Snapshot (if auto_snapshot_on_attach) or Changeset
                ensure!(
                    message.message_type == MessageType::Changeset
                        || message.message_type == MessageType::Snapshot,
                    "Expected Changeset or Snapshot type"
                );
                ensure!(
                    message.signer.starts_with("0x"),
                    "Signer should be hex address"
                );
                ensure!(
                    message.signature.starts_with("0x"),
                    "Signature should be hex"
                );
                ensure!(
                    message.message_hash.starts_with("0x"),
                    "Message hash should be hex"
                );

                Ok(())
            })
            .await
    }

    /// Test: COSE protected header is present in messages (CBOR format)
    pub(crate) async fn test_cose_protected_header(&self) -> TestCaseResult {
        TestCase::new(
            "cose_protected_header",
            "COSE protected header present in messages",
        )
        .run(|| async {
            // Fetch a message and verify it has the COSE protected header
            let message = self.sequencer.fetch_storage_message(0).await?;

            // All messages should have COSE protected header
            ensure!(
                !message.cose_protected_header.is_empty(),
                "Expected cose_protected_header to be present for CBOR format"
            );

            Ok(())
        })
        .await
    }

    /// Test: Signatures are 64-byte COSE format (r || s, no recovery byte)
    pub(crate) async fn test_cose_signature_format(&self) -> TestCaseResult {
        TestCase::new(
            "cose_signature_format",
            "Signatures use 64-byte COSE format",
        )
        .run(|| async {
            let message = self.sequencer.fetch_storage_message(0).await?;

            // COSE signatures are 64 bytes (r || s), not 65 bytes (legacy with recovery)
            // Hex format: 0x + 128 chars = 130 total
            let sig = &message.signature;
            ensure!(sig.starts_with("0x"), "Signature should start with 0x");

            let sig_bytes_len = (sig.len() - 2) / 2; // Remove 0x prefix, convert hex chars to bytes
            ensure!(
                sig_bytes_len == 64,
                "Expected 64-byte COSE signature (128 hex chars), got {} bytes ({} hex chars)",
                sig_bytes_len,
                sig.len() - 2
            );

            Ok(())
        })
        .await
    }

    /// Test: Can fetch raw CBOR batches
    pub(crate) async fn test_cbor_batch_fetch(&self) -> TestCaseResult {
        TestCase::new("cbor_batch_fetch", "Can fetch raw CBOR+zstd batches")
            .run(|| async {
                // List batches
                let batches = self.sequencer.list_batches().await?;
                ensure!(!batches.is_empty(), "Expected at least one batch");

                // Fetch raw CBOR for first batch
                let cbor_data = self
                    .sequencer
                    .fetch_batch_cbor(batches[0].start_sequence)
                    .await?;
                ensure!(!cbor_data.is_empty(), "CBOR batch data should not be empty");

                // Verify it's valid zstd-compressed data (magic number: 0x28 0xB5 0x2F 0xFD)
                ensure!(cbor_data.len() >= 4, "CBOR data too short for zstd header");
                ensure!(
                    cbor_data[0] == 0x28
                        && cbor_data[1] == 0xB5
                        && cbor_data[2] == 0x2F
                        && cbor_data[3] == 0xFD,
                    "Expected zstd magic number in CBOR batch"
                );

                Ok(())
            })
            .await
    }
}
