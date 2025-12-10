use anyhow::Result;
use std::future::Future;
use tracing::{error, info, warn};

/// Result of running all tests
#[derive(Debug)]
pub(crate) struct TestResult {
    pub passed: u32,
    pub failed: u32,
    pub failures: Vec<(String, String)>,
}

impl TestResult {
    pub(crate) fn from_results(results: Vec<TestCaseResult>) -> Self {
        let passed = results.iter().filter(|r| r.is_pass()).count() as u32;
        let failed = results.iter().filter(|r| r.is_fail()).count() as u32;
        let failures: Vec<(String, String)> = results
            .iter()
            .filter_map(|r| {
                if let TestCaseResult::Fail(name, reason) = r {
                    Some((name.clone(), reason.clone()))
                } else {
                    None
                }
            })
            .collect();
        Self {
            passed,
            failed,
            failures,
        }
    }

    pub(crate) fn print_summary(&self) {
        info!("");
        info!("==================================");
        info!("  Test Summary");
        info!("==================================");
        info!("  Passed: {}", self.passed);
        info!("  Failed: {}", self.failed);

        if !self.failures.is_empty() {
            info!("");
            warn!("  Failed tests:");
            for (name, reason) in &self.failures {
                warn!("    - {}: {}", name, reason);
            }
        }
        info!("==================================");
    }
}

/// Result of a single test case
#[derive(Debug)]
pub(crate) enum TestCaseResult {
    Pass(String),
    Fail(String, String),
}

impl TestCaseResult {
    pub(crate) const fn is_pass(&self) -> bool {
        matches!(self, Self::Pass(_))
    }

    pub(crate) const fn is_fail(&self) -> bool {
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
        info!("[TEST] {}: {}", self.name, self.description);

        match f().await {
            Ok(()) => {
                info!("[PASS] {}", self.name);
                TestCaseResult::Pass(self.name)
            }
            Err(e) => {
                let error_msg = format!("{:#}", e);
                error!("[FAIL] {}: {}", self.name, error_msg);
                TestCaseResult::Fail(self.name, error_msg)
            }
        }
    }
}
