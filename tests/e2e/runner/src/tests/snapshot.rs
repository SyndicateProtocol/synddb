use anyhow::ensure;
use tracing::info;

use crate::result::{TestCase, TestCaseResult};
use crate::runner::TestRunner;

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

    /// Test that snapshot appears in DA and can be fetched
    pub(crate) async fn test_snapshot_in_da(&self) -> TestCaseResult {
        TestCase::new("snapshot_in_da", "Snapshot appears in DA layer")
            .run(|| async {
                // Get current DA latest
                let da_before = self.sequencer.da_latest().await?;
                let seq_before = da_before.sequence.unwrap_or(0);

                // Send a snapshot
                let snapshot_data = b"SQLite format 3\x00snapshot for DA test";
                let response = self
                    .sequencer
                    .send_snapshot("e2e-snapshot-da", snapshot_data, 1000)
                    .await?;

                // Small delay for DA propagation
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                // Fetch the message from DA
                let message = self.sequencer.fetch_da_message(response.sequence).await?;

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

                // Verify DA latest updated
                let da_after = self.sequencer.da_latest().await?;
                let seq_after = da_after.sequence.unwrap_or(0);

                ensure!(
                    seq_after > seq_before,
                    "DA latest did not update: before={}, after={}",
                    seq_before,
                    seq_after
                );

                info!(
                    sequence = response.sequence,
                    "Snapshot found in DA with correct message type"
                );

                Ok(())
            })
            .await
    }
}
