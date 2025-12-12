use std::process::ExitCode;

use crate::{
    client::{sequencer::SequencerClient, validator::ValidatorClient},
    runner::TestRunner,
};
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

pub mod client;
pub mod result;
pub mod runner;
pub mod tests;

#[derive(Parser, Debug)]
#[command(name = "synddb-e2e", about = "End-to-end test runner for SyndDB")]
pub struct Config {
    /// Sequencer URL
    #[arg(long, env = "SEQUENCER_URL", default_value = "http://localhost:8433")]
    pub sequencer_url: String,

    /// Primary validator URL
    #[arg(long, env = "VALIDATOR_URL", default_value = "http://localhost:8080")]
    pub validator_url: String,

    /// Secondary validator URL (for multi-validator tests)
    #[arg(long, env = "VALIDATOR2_URL", default_value = "http://localhost:8081")]
    pub validator2_url: String,

    /// Seconds to wait for services to be ready
    #[arg(long, env = "STARTUP_WAIT", default_value = "5")]
    pub startup_wait: u64,

    /// Seconds to wait for data generation
    #[arg(long, env = "DATA_WAIT", default_value = "25")]
    pub data_wait: u64,

    /// Maximum allowed sync difference between sequencer and validator
    #[arg(long, env = "MAX_SYNC_DIFF", default_value = "2")]
    pub max_sync_diff: u64,

    /// Skip DA API tests (for external DA layers like GCS that don't expose /da/* endpoints)
    #[arg(long, env = "SKIP_DA_TESTS", default_value = "false")]
    pub skip_da_tests: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("synddb_e2e=info".parse().unwrap()),
        )
        .init();

    let config = Config::parse();

    info!("==================================");
    info!("  SyndDB E2E Smoke Test");
    info!("==================================");
    info!("");
    info!(
        sequencer = %config.sequencer_url,
        validator = %config.validator_url,
        validator2 = %config.validator2_url,
        "Configuration"
    );

    let sequencer = SequencerClient::new(&config.sequencer_url);
    let validator = ValidatorClient::new(&config.validator_url);
    let validator2 = ValidatorClient::new(&config.validator2_url);

    let runner = TestRunner::new(config, sequencer, validator, validator2);

    match runner.run().await {
        Ok(result) => {
            result.print_summary();

            if result.failed > 0 {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            tracing::error!("Test runner error: {e}");
            ExitCode::FAILURE
        }
    }
}
