use std::time::Duration;

use anyhow::Result;
use tracing::info;

use crate::client::{SequencerClient, ValidatorClient};
use crate::result::TestResult;
use crate::Config;

/// Test runner for E2E tests
pub(crate) struct TestRunner {
    pub config: Config,
    pub sequencer: SequencerClient,
    pub validator: ValidatorClient,
}

impl TestRunner {
    pub(crate) fn new(
        config: Config,
        sequencer: SequencerClient,
        validator: ValidatorClient,
    ) -> Self {
        Self {
            config,
            sequencer,
            validator,
        }
    }

    /// Run all E2E tests
    pub(crate) async fn run(&self) -> Result<TestResult> {
        // Wait for services to be ready
        self.wait_for_services().await?;

        info!("");
        info!("--- Running Tests ---");

        // Run all test cases
        let results = vec![
            self.test_sequencer_receives_messages().await,
            self.test_validator_syncs().await,
            self.test_sync_consistency().await,
            self.test_da_fetch().await,
            self.test_snapshot_sequenced().await,
            self.test_snapshot_in_da().await,
        ];

        Ok(TestResult::from_results(results))
    }

    async fn wait_for_services(&self) -> Result<()> {
        info!("Waiting for services to be ready...");

        let timeout = Duration::from_secs(self.config.startup_wait);
        self.sequencer.wait_healthy(timeout).await?;
        info!("  Sequencer is healthy");

        self.validator.wait_healthy(timeout).await?;
        info!("  Validator is healthy");

        info!(
            "Waiting {}s for customer app to generate data...",
            self.config.data_wait
        );
        tokio::time::sleep(Duration::from_secs(self.config.data_wait)).await;

        Ok(())
    }
}
