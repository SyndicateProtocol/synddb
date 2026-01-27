//! SP1 script for GCP Confidential Space attestation proof generation
//!
//! This script reads attestation samples and generates ZK proofs that can be
//! verified on-chain.

use alloy::{primitives::keccak256, sol_types::SolValue};
use clap::Parser;
use gcp_confidential_space::{
    extract_kid_from_jwt, verify_attestation, JwkKey, PublicValuesStruct,
};
use serde::Deserialize;
use sp1_sdk::{include_elf, ProverClient, SP1Stdin};
use std::fs;

/// The ELF file for the GCP CS attestation verification program
pub const GCP_CS_ATTESTATION_ELF: &[u8] = include_elf!("gcp-cs-attestation-sp1-program");

#[derive(Parser, Debug)]
#[command(author, version, about = "GCP Confidential Space attestation prover")]
struct Args {
    /// Execute the program (test mode, no proof)
    #[arg(long)]
    execute: bool,

    /// Generate a ZK proof
    #[arg(long)]
    prove: bool,

    /// Path to the attestation sample JSON file
    #[arg(long, default_value = "../../samples/samples_1764707758.json")]
    sample: String,

    /// Which sample index to use (0-based)
    #[arg(long, default_value = "0")]
    index: usize,
}

/// Attestation bundle from the sample capture workload
#[derive(Debug, Deserialize)]
struct AttestationBundle {
    samples: Vec<AttestationSample>,
    jwks: Jwks,
}

#[derive(Debug, Deserialize)]
struct AttestationSample {
    raw_token: String,
    audience: String,
}

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<JwkKey>,
}

fn main() {
    sp1_sdk::utils::setup_logger();

    let args = Args::parse();

    if args.execute == args.prove {
        eprintln!("Error: You must specify either --execute or --prove");
        std::process::exit(1);
    }

    // Load the attestation sample
    println!("Loading sample from: {}", args.sample);
    let content = fs::read_to_string(&args.sample).expect("Failed to read sample file");
    let bundle: AttestationBundle =
        serde_json::from_str(&content).expect("Failed to parse sample JSON");

    if args.index >= bundle.samples.len() {
        eprintln!(
            "Error: Sample index {} out of range (have {} samples)",
            args.index,
            bundle.samples.len()
        );
        std::process::exit(1);
    }

    let sample = &bundle.samples[args.index];
    println!("Using sample {} with audience: {}", args.index, sample.audience);

    // Find the JWK for this token (extract kid from JWT header)
    let kid = extract_kid_from_jwt(sample.raw_token.as_bytes())
        .expect("Failed to extract kid from JWT");
    println!("Token signed with key ID: {}", kid);

    let jwk = bundle
        .jwks
        .keys
        .iter()
        .find(|k| k.kid == kid)
        .expect("JWK not found for token's key ID")
        .clone();

    // Setup prover client
    let client = ProverClient::from_env();

    // Prepare inputs for the zkVM
    let mut stdin = SP1Stdin::new();
    stdin.write(&sample.raw_token.as_bytes().to_vec());
    stdin.write(&jwk);
    stdin.write(&sample.audience);

    if args.execute {
        println!("\n=== Executing program (test mode) ===\n");

        let (output, report) = client
            .execute(GCP_CS_ATTESTATION_ELF, &stdin)
            .run()
            .expect("Execution failed");

        println!("Program executed successfully!");

        // Decode the public values
        let public_values =
            PublicValuesStruct::abi_decode_validate(output.as_slice()).expect("Failed to decode output");

        println!("\n=== Public Values ===");
        println!("jwk_key_hash: 0x{}", hex::encode(public_values.jwk_key_hash));
        println!("validity_window_start: {}", public_values.validity_window_start);
        println!("validity_window_end: {}", public_values.validity_window_end);
        println!("image_digest_hash: 0x{}", hex::encode(public_values.image_digest_hash));
        println!("tee_signing_key: {}", public_values.tee_signing_key);
        println!("secboot: {}", public_values.secboot);
        println!("audience_hash: 0x{}", hex::encode(public_values.audience_hash));

        // Verify against local verification
        let expected = verify_attestation(
            sample.raw_token.as_bytes(),
            &jwk,
            Some(&sample.audience),
            None,
        )
        .expect("Local verification failed");

        assert_eq!(
            public_values.jwk_key_hash,
            keccak256(expected.signing_key_id.as_bytes())
        );
        assert_eq!(public_values.validity_window_start, expected.validity_window_start);
        assert_eq!(public_values.validity_window_end, expected.validity_window_end);
        assert_eq!(
            public_values.image_digest_hash,
            keccak256(expected.image_digest.as_bytes())
        );
        assert_eq!(public_values.secboot, expected.secboot);
        assert_eq!(
            public_values.audience_hash,
            keccak256(expected.audience.as_bytes())
        );

        println!("\nAll values match local verification!");
        println!("Cycles executed: {}", report.total_instruction_count());
    } else {
        println!("\n=== Generating ZK proof ===\n");

        let (pk, vk) = client.setup(GCP_CS_ATTESTATION_ELF);

        let proof = client
            .prove(&pk, &stdin)
            .run()
            .expect("Proof generation failed");

        println!("Proof generated successfully!");

        // Verify the proof
        client.verify(&proof, &vk).expect("Proof verification failed");
        println!("Proof verified successfully!");

        // Save proof to file
        let proof_path = "gcp_cs_attestation_proof.bin";
        proof
            .save(proof_path)
            .expect("Failed to save proof");
        println!("Proof saved to: {}", proof_path);
    }
}
