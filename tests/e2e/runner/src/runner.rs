use std::time::Duration;

use crate::{
    client::{sequencer::SequencerClient, validator::ValidatorClient},
    result::TestResult,
    Config,
};
use anyhow::Result;
use tracing::info;

/// Test runner for E2E tests
#[derive(Debug)]
pub struct TestRunner {
    pub config: Config,
    pub sequencer: SequencerClient,
    pub validator: ValidatorClient,
    pub validator2: ValidatorClient,
}

impl TestRunner {
    pub(crate) const fn new(
        config: Config,
        sequencer: SequencerClient,
        validator: ValidatorClient,
        validator2: ValidatorClient,
    ) -> Self {
        Self {
            config,
            sequencer,
            validator,
            validator2,
        }
    }

    /// Run all E2E tests
    pub(crate) async fn run(&self) -> Result<TestResult> {
        // Wait for services to be ready
        self.wait_for_services().await?;

        info!("");
        info!("--- Running Tests ---");

        // Core tests that work with any DA layer
        let mut results = vec![
            self.test_sequencer_receives_messages().await,
            self.test_validator_syncs().await,
            self.test_sync_consistency().await,
            self.test_snapshot_sequenced().await,
            // Multi-validator tests
            self.test_multi_validator_sync().await,
            self.test_validators_consistent().await,
        ];

        // TODO CLAUDE fix this
        // DA API tests (only for LocalPublisher, skip for external DA layers like GCS)
        if self.config.skip_da_tests {
            info!("");
            info!("--- Skipping DA API Tests (SKIP_DA_TESTS=true) ---");
        } else {
            info!("");
            info!("--- Running DA API Tests ---");
            results.extend(vec![
                self.test_storage_fetch().await,
                self.test_snapshot_in_storage().await,
                // Batch storage tests
                self.test_storage_batch_list().await,
                self.test_storage_batch_fetch().await,
                self.test_storage_message_not_found().await,
                self.test_storage_batch_not_found().await,
                self.test_storage_batch_message_consistency().await,
            ]);
        }

        Ok(TestResult::from_results(results))
    }

    async fn wait_for_services(&self) -> Result<()> {
        info!("Waiting for services to be ready...");

        let timeout = Duration::from_secs(self.config.startup_wait);
        self.sequencer.wait_healthy(timeout).await?;
        info!("  Sequencer is healthy");

        self.validator.wait_healthy(timeout).await?;
        info!("  Validator 1 is healthy");

        self.validator2.wait_healthy(timeout).await?;
        info!("  Validator 2 is healthy");

        info!(
            "Waiting {}s for customer app to generate data...",
            self.config.data_wait
        );
        tokio::time::sleep(Duration::from_secs(self.config.data_wait)).await;

        Ok(())
    }
}
