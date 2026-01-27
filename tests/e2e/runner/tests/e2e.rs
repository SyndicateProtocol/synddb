//! E2E Smoke Test
//!
//! Runs end-to-end tests for the `SyndDB` local publisher integration.
//!
//! Usage: `cargo test -p synddb-e2e`

use std::{
    path::PathBuf,
    process::{Command, Output, Stdio},
    thread,
    time::Duration,
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

fn run_docker_compose_output(args: &[&str]) -> Result<Output> {
    let mut cmd = Command::new("docker");
    cmd.arg("compose");
    cmd.args(args);

    cmd.output()
        .context("Failed to run docker compose - is Docker installed and running?")
}

/// Get the container ID for a service
fn get_container_id(compose_file: &str, service: &str) -> Result<String> {
    let output = run_docker_compose_output(&["-f", compose_file, "ps", "-q", service])?;

    let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if container_id.is_empty() {
        bail!("Container for service '{}' not found", service);
    }
    Ok(container_id)
}

/// Wait for a specific container to exit and return its exit code
fn wait_for_container(container_id: &str, timeout: Duration) -> Result<i32> {
    let start = std::time::Instant::now();

    loop {
        // Check if container is still running
        let output = Command::new("docker")
            .args(["inspect", "-f", "{{.State.Running}}", container_id])
            .output()
            .context("Failed to inspect container")?;

        let running = String::from_utf8_lossy(&output.stdout).trim() == "true";

        if !running {
            // Container has exited, get its exit code
            let output = Command::new("docker")
                .args(["inspect", "-f", "{{.State.ExitCode}}", container_id])
                .output()
                .context("Failed to get container exit code")?;

            let exit_code: i32 = String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse()
                .context("Failed to parse exit code")?;

            return Ok(exit_code);
        }

        if start.elapsed() > timeout {
            bail!(
                "Timeout waiting for container {} to exit after {:?}",
                container_id,
                timeout
            );
        }

        thread::sleep(Duration::from_secs(1));
    }
}

#[test]
fn test_e2e_local_publisher() {
    println!("=== SyndDB E2E Tests (Local Publisher) ===");
    println!();

    let project_root = find_project_root().expect("Failed to find project root");
    let compose_file = project_root.join("tests/e2e/docker-compose.yml");
    let compose_file_str = compose_file.to_str().unwrap();

    // Start all containers in detached mode
    println!("Building and starting containers...");
    let build_result = run_docker_compose(&["-f", compose_file_str, "up", "-d", "--build"]);

    if build_result.is_err() || build_result.as_ref().is_ok_and(|&c| c != 0) {
        println!("Failed to start containers, cleaning up...");
        let _ = run_docker_compose(&["-f", compose_file_str, "down", "-v"]);
        panic!(
            "Failed to start containers: {:?}",
            build_result.unwrap_or(1)
        );
    }

    // Get the container ID for e2e_assertions
    println!("Waiting for e2e_assertions container to complete...");
    let container_id = match get_container_id(compose_file_str, "e2e_assertions") {
        Ok(id) => id,
        Err(e) => {
            println!("Failed to get container ID: {}", e);
            let _ = run_docker_compose(&["-f", compose_file_str, "down", "-v"]);
            panic!("Failed to get e2e_assertions container ID");
        }
    };

    // Wait for the assertions container to exit (timeout: 5 minutes)
    let exit_code = match wait_for_container(&container_id, Duration::from_secs(300)) {
        Ok(code) => code,
        Err(e) => {
            println!("Error waiting for container: {}", e);
            // Show logs before cleanup
            println!();
            println!("=== Container logs ===");
            let _ = run_docker_compose(&["-f", compose_file_str, "logs", "e2e_assertions"]);
            let _ = run_docker_compose(&["-f", compose_file_str, "down", "-v"]);
            panic!("Failed to wait for e2e_assertions: {}", e);
        }
    };

    // Show assertion container logs if there was a failure
    if exit_code != 0 {
        println!();
        println!("=== e2e_assertions logs ===");
        let _ = run_docker_compose(&["-f", compose_file_str, "logs", "e2e_assertions"]);
    }

    // Cleanup
    println!();
    println!("Cleaning up containers...");
    let _ = run_docker_compose(&["-f", compose_file_str, "down", "-v"]);

    assert_eq!(exit_code, 0, "E2E tests failed with exit code {exit_code}");
}
