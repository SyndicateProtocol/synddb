use anyhow::ensure;
use tracing::info;

use crate::result::{TestCase, TestCaseResult};
use crate::runner::TestRunner;

impl TestRunner {
    /// Test that both validators independently sync from the sequencer
    pub(crate) async fn test_multi_validator_sync(&self) -> TestCaseResult {
        TestCase::new(
            "multi_validator_sync",
            "Both validators sync from sequencer",
        )
        .run(|| async {
            let v1_status = self.validator.status().await?;
            let v2_status = self.validator2.status().await?;

            let v1_seq = v1_status.last_sequence.unwrap_or(0);
            let v2_seq = v2_status.last_sequence.unwrap_or(0);

            info!(
                validator1_seq = v1_seq,
                validator2_seq = v2_seq,
                "Both validators synced"
            );

            ensure!(
                v1_seq >= 1,
                "Validator 1 has not synced any messages: sequence={}",
                v1_seq
            );

            ensure!(
                v2_seq >= 1,
                "Validator 2 has not synced any messages: sequence={}",
                v2_seq
            );

            Ok(())
        })
        .await
    }

    /// Test that both validators have consistent state with each other
    pub(crate) async fn test_validators_consistent(&self) -> TestCaseResult {
        let max_diff = self.config.max_sync_diff;

        TestCase::new(
            "validators_consistent",
            "Both validators have consistent state",
        )
        .run(|| async {
            let seq_status = self.sequencer.status().await?;
            let v1_status = self.validator.status().await?;
            let v2_status = self.validator2.status().await?;

            let seq_num = seq_status.current_sequence;
            let v1_seq = v1_status.last_sequence.unwrap_or(0);
            let v2_seq = v2_status.last_sequence.unwrap_or(0);

            info!(
                sequencer = seq_num,
                validator1 = v1_seq,
                validator2 = v2_seq,
                "Checking consistency across all services"
            );

            // Check validator 1 is caught up with sequencer
            let v1_diff = seq_num.abs_diff(v1_seq);
            ensure!(
                v1_diff <= max_diff,
                "Validator 1 sync diff too large: sequencer={}, validator1={}, diff={} (max={})",
                seq_num,
                v1_seq,
                v1_diff,
                max_diff
            );

            // Check validator 2 is caught up with sequencer
            let v2_diff = seq_num.abs_diff(v2_seq);
            ensure!(
                v2_diff <= max_diff,
                "Validator 2 sync diff too large: sequencer={}, validator2={}, diff={} (max={})",
                seq_num,
                v2_seq,
                v2_diff,
                max_diff
            );

            // Check both validators are consistent with each other
            let inter_validator_diff = v1_seq.abs_diff(v2_seq);
            ensure!(
                inter_validator_diff <= max_diff,
                "Validators inconsistent: validator1={}, validator2={}, diff={} (max={})",
                v1_seq,
                v2_seq,
                inter_validator_diff,
                max_diff
            );

            Ok(())
        })
        .await
    }
}
