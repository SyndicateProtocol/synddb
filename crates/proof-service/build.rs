//! Build script for proof-service
//!
//! Compiles the RISC Zero program for GCP Confidential Space attestation verification
//! and extracts the image ID to a file for CI/deployment use.

use std::fs;
use std::path::Path;

fn main() {
    // Verify parsing logic works before building (fail fast if broken)
    verify_parsing();

    // Build the RISC Zero guest program from the bootstrap crate
    risc0_build::embed_methods();

    // Extract and save the image ID for CI/deployment
    // This avoids needing to run the CUDA binary just to read a compile-time constant
    if let Err(e) = extract_image_id() {
        println!("cargo:warning=Failed to extract image ID: {}", e);
    }
}

/// Extract the RISC Zero image ID from the generated methods.rs and save it to a file.
///
/// The image ID is needed by CI to attach as an OCI artifact and by Terraform to
/// configure the on-chain verifier contract. By extracting it during build, we avoid
/// needing CUDA stubs or a GPU to run the binary.
fn extract_image_id() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::env::var("OUT_DIR")?;
    let methods_path = Path::new(&out_dir).join("methods.rs");

    let methods_content = fs::read_to_string(&methods_path)?;

    // Parse the image ID from the generated methods.rs
    // Format: pub const GCP_CS_ATTESTATION_RISC0_PROGRAM_ID: [u32; 8] = [...]
    let id = parse_image_id(&methods_content)?;

    // Convert [u32; 8] to bytes32 hex string (same logic as risc0_image_id_bytes32)
    let mut bytes = [0u8; 32];
    for (i, word) in id.iter().enumerate() {
        bytes[i * 4..(i + 1) * 4].copy_from_slice(&word.to_le_bytes());
    }
    let hex_string = format!("0x{}", hex::encode(bytes));

    // Write to the crate directory so it can be easily copied in Dockerfile
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
    let output_path = Path::new(&manifest_dir).join("risc0_image_id.txt");
    fs::write(&output_path, &hex_string)?;

    println!("cargo:warning=RISC Zero image ID: {}", hex_string);
    println!("cargo:rerun-if-changed={}", methods_path.display());

    Ok(())
}

/// Parse the image ID constant from generated methods.rs content.
/// Example format: `pub const GCP_CS_ATTESTATION_RISC0_PROGRAM_ID: [u32; 8] = [123, 456, ...];`
fn parse_image_id(content: &str) -> Result<[u32; 8], Box<dyn std::error::Error>> {
    let line = content
        .lines()
        .find(|l| l.contains("RISC0_PROGRAM_ID"))
        .ok_or("RISC0_PROGRAM_ID not found in methods.rs")?;

    let array_part = line.split('=').nth(1).ok_or("No '=' found in PROGRAM_ID line")?;
    let inner = array_part
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(';')
        .trim_end_matches(']');

    let nums: Vec<u32> = inner
        .split(',')
        .map(|s| s.trim().parse())
        .collect::<Result<_, _>>()?;

    nums.try_into()
        .map_err(|_| "RISC0_PROGRAM_ID should have exactly 8 elements".into())
}

/// Verify parsing works with the expected format by testing against a known production value.
/// This runs at build time - if parsing is broken, the build fails here.
fn verify_parsing() {
    // Simulated methods.rs content matching risc0-build output format
    // Uses a real image ID (0xe077c51b...) that was deployed to Base Sepolia
    let test_content = r#"
pub const GCP_CS_ATTESTATION_RISC0_PROGRAM_ID: [u32; 8] = [465926112, 3735277551, 508429070, 3693580442, 1954273035, 1026983289, 3238576303, 2251715051];
"#;

    let id = parse_image_id(test_content).expect("Parsing verification failed");

    // Convert to hex and verify
    let mut bytes = [0u8; 32];
    for (i, word) in id.iter().enumerate() {
        bytes[i * 4..(i + 1) * 4].copy_from_slice(&word.to_le_bytes());
    }
    let hex_output = format!("0x{}", hex::encode(bytes));

    assert_eq!(
        hex_output,
        "0xe077c51befcfa3de0e034e1e9a9027dc0bd77b747985363dafc008c1eb713686",
        "Image ID parsing produced unexpected result"
    );
}
