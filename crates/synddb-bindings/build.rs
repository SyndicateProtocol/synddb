use std::{env, fs, path::Path, process::Command};

fn main() {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");

    // contracts/ is at the workspace root, two levels up from this crate
    let workspace_root = Path::new(&manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .expect("Failed to find workspace root");
    let contracts_dir = workspace_root.join("contracts");

    // Rerun if any Solidity source files change
    println!(
        "cargo::rerun-if-changed={}",
        contracts_dir.join("src").display()
    );

    // Run forge bind to generate bindings as a single module file
    let status = Command::new("forge")
        .current_dir(&contracts_dir)
        .args([
            "bind",
            "--module",
            "--single-file",
            "--overwrite",
            "-b",
            &out_dir,
        ])
        .status()
        .expect("Failed to run forge bind. Is Foundry installed?");

    if !status.success() {
        panic!("forge bind failed with status: {}", status);
    }

    // Post-process the generated file to make it compatible with include!()
    // The generated file has inner attributes (#![...]) and inner doc comments (//!)
    // which are not allowed when using include!() - they must be at crate root.
    let generated_file = Path::new(&out_dir).join("mod.rs");
    let content = fs::read_to_string(&generated_file).expect("Failed to read generated bindings");

    let processed: String = content
        .lines()
        .map(|line| {
            if line.starts_with("#![") {
                // Convert inner attribute to outer attribute comment (disable it)
                format!("// {line}")
            } else if line.starts_with("//!") {
                // Convert inner doc comment to regular comment
                format!("//{}", &line[3..])
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    fs::write(&generated_file, processed).expect("Failed to write processed bindings");
}
