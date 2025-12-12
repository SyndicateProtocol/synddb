use anyhow::ensure;

use crate::{
    result::{TestCase, TestCaseResult},
    runner::TestRunner,
};

impl TestRunner {
    /// Test that the validator is caught up with the sequencer
    pub(crate) async fn test_sync_consistency(&self) -> TestCaseResult {
        let max_diff = self.config.max_sync_diff;

        TestCase::new("sync_consistency", "Validator caught up with sequencer")
            .run(|| async {
                let seq_status = self.sequencer.status().await?;
                let val_status = self.validator.status().await?;

                let seq_num = seq_status.current_sequence;
                let val_num = val_status.last_sequence.unwrap_or(0);
                let diff = seq_num.abs_diff(val_num);

                ensure!(
                    diff <= max_diff,
                    "Sync diff too large: sequencer={}, validator={}, diff={} (max={})",
                    seq_num,
                    val_num,
                    diff,
                    max_diff
                );

                Ok(())
            })
            .await
    }
}
