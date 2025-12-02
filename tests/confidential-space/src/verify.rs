//! Local verification test for Confidential Space attestation samples
//!
//! This binary verifies that captured attestation tokens have valid signatures
//! from Google's Confidential Space signing keys.
//!
//! Usage:
//!   cargo run --bin verify-sample [path/to/sample.json]
//!
//! If no path is provided, it looks for samples in ./samples/

use anyhow::{Context, Result};
use base64::Engine;
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::signature::Verifier;
use rsa::{BigUint, RsaPublicKey};
use serde::Deserialize;
use sha2::Sha256;
use std::path::PathBuf;

/// Attestation sample structure (matches output from cs-attestation-sample)
#[derive(Debug, Deserialize)]
struct AttestationBundle {
    samples: Vec<AttestationSample>,
    jwks: Jwks,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AttestationSample {
    raw_token: String,
    header: JwtHeader,
    claims: serde_json::Value,
    signature_bytes: String,
    signing_input: String,
    audience: String,
    nonces: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JwtHeader {
    alg: String,
    kid: String,
    typ: String,
}

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<JwkKey>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JwkKey {
    alg: String,
    kid: String,
    kty: String,
    n: String,
    e: String,
    #[serde(rename = "use")]
    use_: String,
}

fn main() -> Result<()> {
    println!("=== Confidential Space Attestation Verifier ===\n");

    // Find sample file
    let sample_path = find_sample_file()?;
    println!("Loading sample: {}\n", sample_path.display());

    // Load and parse the bundle
    let content = std::fs::read_to_string(&sample_path)
        .with_context(|| format!("Failed to read {}", sample_path.display()))?;
    let bundle: AttestationBundle =
        serde_json::from_str(&content).context("Failed to parse sample JSON")?;

    println!(
        "Found {} sample(s) and {} JWKS key(s)\n",
        bundle.samples.len(),
        bundle.jwks.keys.len()
    );

    // Verify each sample
    let mut all_passed = true;
    for (i, sample) in bundle.samples.iter().enumerate() {
        println!("--- Sample {} ---", i + 1);
        println!("  Audience: {}", sample.audience);
        println!("  Nonces: {:?}", sample.nonces);
        println!("  Algorithm: {}", sample.header.alg);
        println!("  Key ID: {}", sample.header.kid);

        match verify_sample(sample, &bundle.jwks) {
            Ok(()) => {
                println!("  Result: VALID\n");
            }
            Err(e) => {
                println!("  Result: INVALID - {}\n", e);
                all_passed = false;
            }
        }

        // Print some interesting claims
        print_claims(&sample.claims);
    }

    if all_passed {
        println!("=== All samples verified successfully ===");
        Ok(())
    } else {
        anyhow::bail!("Some samples failed verification")
    }
}

fn find_sample_file() -> Result<PathBuf> {
    // Check command line argument first
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let path = PathBuf::from(&args[1]);
        if path.exists() {
            return Ok(path);
        } else {
            anyhow::bail!("Specified file not found: {}", path.display());
        }
    }

    // Look in ./samples/ directory
    let samples_dir = PathBuf::from("samples");
    if samples_dir.exists() {
        for entry in std::fs::read_dir(&samples_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                return Ok(path);
            }
        }
    }

    // Try current directory
    for entry in std::fs::read_dir(".")? {
        let entry = entry?;
        let path = entry.path();
        if path
            .file_name()
            .map(|n| n.to_string_lossy().starts_with("samples_"))
            .unwrap_or(false)
        {
            return Ok(path);
        }
    }

    anyhow::bail!(
        "No sample file found. Run from tests/confidential-space/ or provide a path as argument."
    )
}

fn verify_sample(sample: &AttestationSample, jwks: &Jwks) -> Result<()> {
    // Check algorithm
    if sample.header.alg != "RS256" {
        anyhow::bail!(
            "Unsupported algorithm: {} (expected RS256)",
            sample.header.alg
        );
    }

    // Find the signing key
    let jwk = jwks
        .keys
        .iter()
        .find(|k| k.kid == sample.header.kid)
        .with_context(|| format!("Key ID {} not found in JWKS", sample.header.kid))?;

    // Build RSA public key from JWK
    let public_key = jwk_to_rsa_public_key(jwk)?;
    let verifying_key = VerifyingKey::<Sha256>::new(public_key);

    // Decode signature from hex
    let sig_hex = sample
        .signature_bytes
        .strip_prefix("0x")
        .unwrap_or(&sample.signature_bytes);
    let signature_bytes = hex::decode(sig_hex).context("Failed to decode signature hex")?;
    let signature = Signature::try_from(signature_bytes.as_slice())
        .context("Failed to parse signature bytes")?;

    // Verify: the signing_input is the message that was signed
    verifying_key
        .verify(sample.signing_input.as_bytes(), &signature)
        .context("Signature verification failed")?;

    Ok(())
}

fn jwk_to_rsa_public_key(jwk: &JwkKey) -> Result<RsaPublicKey> {
    if jwk.kty != "RSA" {
        anyhow::bail!("Expected RSA key type, got {}", jwk.kty);
    }

    // Decode n (modulus) and e (exponent) from base64url
    let n_bytes = decode_base64url(&jwk.n).context("Failed to decode modulus (n)")?;
    let e_bytes = decode_base64url(&jwk.e).context("Failed to decode exponent (e)")?;

    let n = BigUint::from_bytes_be(&n_bytes);
    let e = BigUint::from_bytes_be(&e_bytes);

    RsaPublicKey::new(n, e).context("Failed to construct RSA public key")
}

fn decode_base64url(input: &str) -> Result<Vec<u8>> {
    // base64url uses - and _ instead of + and /
    // Also may lack padding
    let padded = match input.len() % 4 {
        2 => format!("{}==", input),
        3 => format!("{}=", input),
        _ => input.to_string(),
    };

    let standard = padded.replace('-', "+").replace('_', "/");

    base64::engine::general_purpose::STANDARD
        .decode(&standard)
        .context("Base64 decode failed")
}

fn print_claims(claims: &serde_json::Value) {
    println!("  Claims:");

    if let Some(iss) = claims.get("iss").and_then(|v| v.as_str()) {
        println!("    iss: {}", iss);
    }
    if let Some(aud) = claims.get("aud").and_then(|v| v.as_str()) {
        println!("    aud: {}", aud);
    }
    if let Some(hwmodel) = claims.get("hwmodel").and_then(|v| v.as_str()) {
        println!("    hwmodel: {}", hwmodel);
    }
    if let Some(swname) = claims.get("swname").and_then(|v| v.as_str()) {
        println!("    swname: {}", swname);
    }
    if let Some(secboot) = claims.get("secboot").and_then(|v| v.as_bool()) {
        println!("    secboot: {}", secboot);
    }
    if let Some(dbgstat) = claims.get("dbgstat").and_then(|v| v.as_str()) {
        println!("    dbgstat: {}", dbgstat);
    }

    // Container image digest
    if let Some(digest) = claims
        .get("submods")
        .and_then(|s| s.get("container"))
        .and_then(|c| c.get("image_digest"))
        .and_then(|v| v.as_str())
    {
        println!("    image_digest: {}", digest);
    }

    println!();
}
