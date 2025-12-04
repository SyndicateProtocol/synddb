use anyhow::Result;
use std::future::Future;
use tracing::{error, info};

/// Result of running all tests
#[derive(Debug)]
pub(crate) struct TestResult {
    pub passed: u32,
    pub failed: u32,
}

impl TestResult {
    pub(crate) fn from_results(results: Vec<TestCaseResult>) -> Self {
        let passed = results.iter().filter(|r| r.is_pass()).count() as u32;
        let failed = results.iter().filter(|r| r.is_fail()).count() as u32;
        Self { passed, failed }
    }
}

/// Result of a single test case
#[derive(Debug)]
pub(crate) enum TestCaseResult {
    Pass(String),
    Fail(String, String),
}

impl TestCaseResult {
    pub(crate) fn is_pass(&self) -> bool {
        matches!(self, Self::Pass(_))
    }

    pub(crate) fn is_fail(&self) -> bool {
        matches!(self, Self::Fail(_, _))
    }
}

/// Helper struct for building and running test cases
pub(crate) struct TestCase {
    name: String,
    description: String,
}

impl TestCase {
    pub(crate) fn new(name: &str, description: &str) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
        }
    }

    pub(crate) async fn run<F, Fut>(self, f: F) -> TestCaseResult
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        info!("{}: {} ...", self.name, self.description);

        match f().await {
            Ok(()) => {
                info!("  PASS");
                TestCaseResult::Pass(self.name)
            }
            Err(e) => {
                error!("  FAIL: {}", e);
                TestCaseResult::Fail(self.name, e.to_string())
            }
        }
    }
}
