use anyhow::ensure;
use tracing::info;

use crate::{
    result::{TestCase, TestCaseResult},
    runner::TestRunner,
};

impl TestRunner {
    /// Test that the sequencer can receive and sequence a snapshot
    pub(crate) async fn test_snapshot_sequenced(&self) -> TestCaseResult {
        TestCase::new("snapshot_sequenced", "Snapshot is sequenced and signed")
            .run(|| async {
                // Create a minimal SQLite database header as test data
                let snapshot_data = b"SQLite format 3\x00test snapshot data for e2e";

                // Get current sequence before sending snapshot
                let status_before = self.sequencer.status().await?;
                let seq_before = status_before.current_sequence;

                // Send snapshot
                let response = self
                    .sequencer
                    .send_snapshot("e2e-snapshot-1", snapshot_data, 999)
                    .await?;

                info!(
                    sequence = response.sequence,
                    message_hash = %response.message_hash,
                    "Snapshot sequenced"
                );

                // Verify the sequence was assigned
                ensure!(
                    response.signature.starts_with("0x"),
                    "Invalid signature format: {}",
                    response.signature
                );

                ensure!(
                    response.message_hash.starts_with("0x"),
                    "Invalid message hash format: {}",
                    response.message_hash
                );

                // Verify sequence incremented
                let status_after = self.sequencer.status().await?;
                ensure!(
                    status_after.current_sequence > seq_before,
                    "Sequence did not increment: before={}, after={}",
                    seq_before,
                    status_after.current_sequence
                );

                Ok(())
            })
            .await
    }

    /// Test that snapshot appears in storage and can be fetched
    pub(crate) async fn test_snapshot_in_storage(&self) -> TestCaseResult {
        TestCase::new("snapshot_in_storage", "Snapshot appears in storage layer")
            .run(|| async {
                // Get current storage latest
                let storage_before = self.sequencer.storage_latest().await?;
                let seq_before = storage_before.sequence.unwrap_or(0);

                // Send a snapshot
                let snapshot_data = b"SQLite format 3\x00snapshot for storage test";
                let response = self
                    .sequencer
                    .send_snapshot("e2e-snapshot-storage", snapshot_data, 1000)
                    .await?;

                // Wait for the message to appear in storage (may need to wait for batch flush)
                // The CBOR batcher flushes every 1s, so we retry for up to 3s
                let mut message = None;
                for _ in 0..30 {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    if let Some(msg) = self
                        .sequencer
                        .try_fetch_storage_message(response.sequence)
                        .await?
                    {
                        message = Some(msg);
                        break;
                    }
                }

                let message = message.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Snapshot at sequence {} did not appear in storage after 3s",
                        response.sequence
                    )
                })?;

                // Verify message type is snapshot
                ensure!(
                    message.message_type == synddb_shared::types::message::MessageType::Snapshot,
                    "Expected snapshot message type, got {:?}",
                    message.message_type
                );

                // Verify sequence matches
                ensure!(
                    message.sequence == response.sequence,
                    "Sequence mismatch: expected {}, got {}",
                    response.sequence,
                    message.sequence
                );

                // Verify storage latest updated
                let storage_after = self.sequencer.storage_latest().await?;
                let seq_after = storage_after.sequence.unwrap_or(0);

                ensure!(
                    seq_after > seq_before,
                    "Storage latest did not update: before={}, after={}",
                    seq_before,
                    seq_after
                );

                info!(
                    sequence = response.sequence,
                    "Snapshot found in storage with correct message type"
                );

                Ok(())
            })
            .await
    }
}
