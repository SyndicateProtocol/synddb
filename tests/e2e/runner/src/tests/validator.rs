use anyhow::ensure;

use crate::{
    result::{TestCase, TestCaseResult},
    runner::TestRunner,
};

impl TestRunner {
    /// Test that the validator has synced messages from the sequencer
    pub(crate) async fn test_validator_syncs(&self) -> TestCaseResult {
        TestCase::new("validator_synced", "Validator synced messages")
            .run(|| async {
                let status = self.validator.status().await?;

                let last_sequence = status.last_sequence.unwrap_or(0);
                ensure!(
                    last_sequence >= 1,
                    "Expected at least 1 synced message, got {}",
                    last_sequence
                );

                Ok(())
            })
            .await
    }
}
