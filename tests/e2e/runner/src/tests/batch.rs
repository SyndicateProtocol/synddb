//! Batch storage layer fetch tests

use anyhow::ensure;
use tracing::info;

use crate::{
    result::{TestCase, TestCaseResult},
    runner::TestRunner,
};

impl TestRunner {
    /// Test: Can list batches from the storage layer
    pub(crate) async fn test_storage_batch_list(&self) -> TestCaseResult {
        TestCase::new("storage_batch_list", "Storage batch list endpoint works")
            .run(|| async {
                let batches = self.sequencer.list_batches().await?;

                ensure!(
                    !batches.is_empty(),
                    "Expected at least one batch in storage"
                );

                // Verify batches are sorted by start_sequence
                for window in batches.windows(2) {
                    ensure!(
                        window[0].start_sequence < window[1].start_sequence,
                        "Batches not sorted: {} >= {}",
                        window[0].start_sequence,
                        window[1].start_sequence
                    );
                }

                // Verify each batch has valid sequence range
                for batch in &batches {
                    ensure!(
                        batch.end_sequence >= batch.start_sequence,
                        "Invalid batch range: start={} > end={}",
                        batch.start_sequence,
                        batch.end_sequence
                    );
                }

                info!(
                    batch_count = batches.len(),
                    first_start = batches.first().map(|b| b.start_sequence),
                    last_end = batches.last().map(|b| b.end_sequence),
                    "Listed batches from storage"
                );

                Ok(())
            })
            .await
    }

    /// Test: Can fetch a batch by start sequence
    pub(crate) async fn test_storage_batch_fetch(&self) -> TestCaseResult {
        TestCase::new("storage_batch_fetch", "Storage batch fetch works")
            .run(|| async {
                // First list batches to get a valid start_sequence
                let batches = self.sequencer.list_batches().await?;
                ensure!(!batches.is_empty(), "No batches available to fetch");

                let batch_info = &batches[0];
                let batch = self
                    .sequencer
                    .fetch_batch(batch_info.start_sequence)
                    .await?;

                // Verify batch metadata matches
                ensure!(
                    batch.start_sequence == batch_info.start_sequence,
                    "Batch start_sequence mismatch: expected {}, got {}",
                    batch_info.start_sequence,
                    batch.start_sequence
                );
                ensure!(
                    batch.end_sequence == batch_info.end_sequence,
                    "Batch end_sequence mismatch: expected {}, got {}",
                    batch_info.end_sequence,
                    batch.end_sequence
                );

                // Verify batch has messages
                ensure!(
                    !batch.messages.is_empty(),
                    "Batch should contain at least one message"
                );

                // Verify message count matches range
                let expected_count = (batch.end_sequence - batch.start_sequence + 1) as usize;
                ensure!(
                    batch.messages.len() == expected_count,
                    "Batch message count mismatch: expected {}, got {}",
                    expected_count,
                    batch.messages.len()
                );

                // Verify messages are in sequence order
                for (i, msg) in batch.messages.iter().enumerate() {
                    let expected_seq = batch.start_sequence + i as u64;
                    ensure!(
                        msg.sequence == expected_seq,
                        "Message sequence mismatch at index {}: expected {}, got {}",
                        i,
                        expected_seq,
                        msg.sequence
                    );
                }

                info!(
                    start = batch.start_sequence,
                    end = batch.end_sequence,
                    messages = batch.messages.len(),
                    "Fetched batch from storage"
                );

                Ok(())
            })
            .await
    }

    /// Test: Fetching a non-existent message returns 404
    pub(crate) async fn test_storage_message_not_found(&self) -> TestCaseResult {
        TestCase::new(
            "storage_message_not_found",
            "Non-existent message returns 404",
        )
        .run(|| async {
            // Get the latest sequence and request one far beyond it
            let latest = self.sequencer.storage_latest().await?;
            let latest_seq = latest.sequence.unwrap_or(0);
            let non_existent_seq = latest_seq + 1_000_000;

            let result = self
                .sequencer
                .try_fetch_storage_message(non_existent_seq)
                .await?;

            ensure!(
                result.is_none(),
                "Expected None for non-existent message at sequence {}, got Some",
                non_existent_seq
            );

            info!(
                sequence = non_existent_seq,
                "Correctly returned 404 for non-existent message"
            );

            Ok(())
        })
        .await
    }

    /// Test: Fetching a non-existent batch returns 404
    pub(crate) async fn test_storage_batch_not_found(&self) -> TestCaseResult {
        TestCase::new("storage_batch_not_found", "Non-existent batch returns 404")
            .run(|| async {
                // Get the latest sequence and request a batch far beyond it
                let latest = self.sequencer.storage_latest().await?;
                let latest_seq = latest.sequence.unwrap_or(0);
                let non_existent_start = latest_seq + 1_000_000;

                let result = self.sequencer.try_fetch_batch(non_existent_start).await?;

                ensure!(
                    result.is_none(),
                    "Expected None for non-existent batch starting at {}, got Some",
                    non_existent_start
                );

                info!(
                    start_sequence = non_existent_start,
                    "Correctly returned 404 for non-existent batch"
                );

                Ok(())
            })
            .await
    }

    /// Test: Messages in batch match individually fetched messages
    pub(crate) async fn test_storage_batch_message_consistency(&self) -> TestCaseResult {
        TestCase::new(
            "storage_batch_message_consistency",
            "Batch messages match individual fetches",
        )
        .run(|| async {
            // Get the first batch
            let batches = self.sequencer.list_batches().await?;
            ensure!(!batches.is_empty(), "No batches available");

            let batch_info = &batches[0];
            let batch = self
                .sequencer
                .fetch_batch(batch_info.start_sequence)
                .await?;

            // Verify at least a few messages match their individual fetches
            let check_count = std::cmp::min(3, batch.messages.len());

            for msg_from_batch in batch.messages.iter().take(check_count) {
                let msg_individual = self
                    .sequencer
                    .fetch_storage_message(msg_from_batch.sequence)
                    .await?;

                // Compare key fields
                ensure!(
                    msg_from_batch.sequence == msg_individual.sequence,
                    "Sequence mismatch: batch={}, individual={}",
                    msg_from_batch.sequence,
                    msg_individual.sequence
                );
                ensure!(
                    msg_from_batch.message_hash == msg_individual.message_hash,
                    "Message hash mismatch at sequence {}: batch={}, individual={}",
                    msg_from_batch.sequence,
                    msg_from_batch.message_hash,
                    msg_individual.message_hash
                );
                ensure!(
                    msg_from_batch.signature == msg_individual.signature,
                    "Signature mismatch at sequence {}: batch={}, individual={}",
                    msg_from_batch.sequence,
                    msg_from_batch.signature,
                    msg_individual.signature
                );
                ensure!(
                    msg_from_batch.signer == msg_individual.signer,
                    "Signer mismatch at sequence {}",
                    msg_from_batch.sequence
                );
                ensure!(
                    msg_from_batch.message_type == msg_individual.message_type,
                    "Message type mismatch at sequence {}",
                    msg_from_batch.sequence
                );
            }

            info!(
                checked = check_count,
                "Batch messages match individual fetches"
            );

            Ok(())
        })
        .await
    }
}
