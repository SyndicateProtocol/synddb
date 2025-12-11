//! GCS E2E Test
//!
//! Runs end-to-end tests for the SyndDB GCS storage layer integration.
//!
//! Modes:
//!   - Local emulator (default): `cargo test -p synddb-e2e-gcs`
//!   - Real GCS: `REAL_GCS=true GCS_BUCKET=... GOOGLE_APPLICATION_CREDENTIALS=... cargo test -p synddb-e2e-gcs`

use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use chrono::Utc;

fn find_project_root() -> Result<PathBuf> {
    let mut current = std::env::current_dir()?;

    loop {
        if current.join("Cargo.toml").exists() && current.join("crates").exists() {
            return Ok(current);
        }

        if !current.pop() {
            bail!("Could not find project root (looking for Cargo.toml with crates/ directory)");
        }
    }
}

fn run_docker_compose(args: &[&str]) -> Result<i32> {
    run_docker_compose_with_env(args, &[])
}

fn run_docker_compose_with_env(args: &[&str], env: &[(&str, &str)]) -> Result<i32> {
    let mut cmd = Command::new("docker");
    cmd.arg("compose");
    cmd.args(args);
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    for (key, value) in env {
        cmd.env(key, value);
    }

    let status = cmd
        .status()
        .context("Failed to run docker compose - is Docker installed and running?")?;

    Ok(status.code().unwrap_or(1))
}

#[test]
fn test_gcs_emulator() {
    run_emulator_test();
}

#[test]
#[ignore] // Requires GCS_BUCKET=... GOOGLE_APPLICATION_CREDENTIALS=...
fn test_gcs_real() {
    run_real_gcs_test();
}

fn run_emulator_test() {
    println!("=== SyndDB GCS E2E Tests (Emulator Mode) ===");
    println!();

    let project_root = find_project_root().expect("Failed to find project root");
    let compose_dir = project_root.join("tests/e2e-gcs");
    let compose_file = compose_dir.join("docker-compose.yml");

    println!("Running with fake-gcs-server emulator...");

    let exit_code = run_docker_compose(&[
        "-f",
        compose_file.to_str().unwrap(),
        "up",
        "--build",
        "--abort-on-container-exit",
        "--exit-code-from",
        "e2e_assertions",
    ])
    .expect("Failed to run docker compose");

    // Cleanup
    println!();
    println!("Cleaning up containers...");
    let _ = run_docker_compose(&["-f", compose_file.to_str().unwrap(), "down", "-v"]);

    assert_eq!(
        exit_code, 0,
        "GCS E2E tests failed with exit code {exit_code}"
    );
}

fn run_real_gcs_test() {
    println!("=== SyndDB GCS E2E Tests (Real GCS Mode) ===");
    println!();

    let project_root = find_project_root().expect("Failed to find project root");

    let bucket = std::env::var("GCS_BUCKET")
        .expect("GCS_BUCKET environment variable must be set for real GCS mode");

    let credentials = std::env::var("GOOGLE_APPLICATION_CREDENTIALS").expect(
        "GOOGLE_APPLICATION_CREDENTIALS environment variable must be set for real GCS mode",
    );

    // Resolve credentials path relative to project root if not absolute
    let credentials_path = PathBuf::from(&credentials);
    let credentials_path = if credentials_path.is_absolute() {
        credentials_path
    } else {
        project_root.join(&credentials_path)
    };
    assert!(
        credentials_path.exists(),
        "Credentials file not found: {}",
        credentials_path.display()
    );
    let credentials_str = credentials_path.to_str().unwrap();

    let test_run_id = std::env::var("TEST_RUN_ID")
        .unwrap_or_else(|_| Utc::now().format("%Y%m%d-%H%M%S").to_string());

    println!("Bucket: {bucket}");
    println!("Prefix: sequencer-test-{test_run_id}");
    println!("Credentials: {credentials_str}");
    println!();

    let compose_dir = project_root.join("tests/e2e-gcs");
    let base_compose = compose_dir.join("docker-compose.yml");
    let real_gcs_compose = compose_dir.join("docker-compose.real-gcs.yml");

    let exit_code = run_docker_compose_with_env(
        &[
            "-f",
            base_compose.to_str().unwrap(),
            "-f",
            real_gcs_compose.to_str().unwrap(),
            "up",
            "--build",
            "--abort-on-container-exit",
            "--exit-code-from",
            "e2e_assertions",
        ],
        &[
            ("GCS_BUCKET", bucket.as_str()),
            ("GOOGLE_APPLICATION_CREDENTIALS", credentials_str),
            ("TEST_RUN_ID", &test_run_id),
        ],
    )
    .expect("Failed to run docker compose");

    // Cleanup
    println!();
    println!("Cleaning up containers...");
    let _ = run_docker_compose_with_env(
        &[
            "-f",
            base_compose.to_str().unwrap(),
            "-f",
            real_gcs_compose.to_str().unwrap(),
            "down",
            "-v",
        ],
        &[
            ("GCS_BUCKET", bucket.as_str()),
            ("GOOGLE_APPLICATION_CREDENTIALS", credentials_str),
            ("TEST_RUN_ID", &test_run_id),
        ],
    );

    println!();
    println!("Test data was written to: gs://{bucket}/sequencer-test-{test_run_id}/");
    println!("To clean up: gsutil -m rm -r gs://{bucket}/sequencer-test-{test_run_id}/");

    assert_eq!(
        exit_code, 0,
        "GCS E2E tests failed with exit code {exit_code}"
    );
}
