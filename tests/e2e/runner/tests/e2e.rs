//! E2E Smoke Test
//!
//! Runs end-to-end tests for the `SyndDB` local publisher integration.
//!
//! Usage: `cargo test -p synddb-e2e`

use std::{
    path::PathBuf,
    process::{Command, Stdio},
};

use anyhow::{bail, Context, Result};

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
    let mut cmd = Command::new("docker");
    cmd.arg("compose");
    cmd.args(args);
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    let status = cmd
        .status()
        .context("Failed to run docker compose - is Docker installed and running?")?;

    Ok(status.code().unwrap_or(1))
}

#[test]
fn test_e2e_local_publisher() {
    println!("=== SyndDB E2E Tests (Local Publisher) ===");
    println!();

    let project_root = find_project_root().expect("Failed to find project root");
    let compose_file = project_root.join("tests/e2e/docker-compose.yml");

    println!("Running E2E smoke test...");

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

    assert_eq!(exit_code, 0, "E2E tests failed with exit code {exit_code}");
}
