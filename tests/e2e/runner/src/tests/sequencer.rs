use anyhow::ensure;

use crate::{
    result::{TestCase, TestCaseResult},
    runner::TestRunner,
};

impl TestRunner {
    /// Test that the sequencer has received messages from the customer app
    pub(crate) async fn test_sequencer_receives_messages(&self) -> TestCaseResult {
        TestCase::new("sequencer_received", "Sequencer received messages")
            .run(|| async {
                let status = self.sequencer.status().await?;

                ensure!(
                    status.current_sequence >= 1,
                    "Expected at least 1 message, got {}",
                    status.current_sequence
                );

                Ok(())
            })
            .await
    }
}
