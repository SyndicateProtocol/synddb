//! Encoding format benchmark
//!
//! Benchmarks CBOR encoding/compression combinations:
//! - CBOR (uncompressed)
//! - CBOR + zstd (production format)

use alloy::{
    primitives::{keccak256, B256},
    signers::{k256::ecdsa::VerifyingKey, local::PrivateKeySigner, SignerSync},
};
use anyhow::Result;
use k256::ecdsa::Signature;
use rusqlite::{session::Session, Connection};
use std::time::Instant;
use synddb_benchmark::schema::initialize_schema;
use synddb_shared::types::cbor::{
    batch::CborBatch,
    error::CborError,
    message::{CborMessageType, CborSignedMessage},
    verify::{signature_from_bytes, verifying_key_from_bytes},
};

/// Test private key (well-known test key, do not use in production)
const TEST_PRIVATE_KEY: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

/// Test case configuration
/// - N (`num_messages`): Number of messages in a batch
/// - M (`changesets_per_message`): Number of changesets per message
/// - O (`ops_per_changeset`): Number of SQL operations per changeset (simulates activity level)
#[derive(Debug, Clone)]
struct TestCase {
    name: &'static str,
    num_messages: usize,
    changesets_per_message: usize,
    ops_per_changeset: usize,
}

/// Results for a single encoding format
#[derive(Debug)]
struct EncodingResult {
    format_name: &'static str,
    encoded_bytes: usize,
    encode_time_us: u64,
    decode_time_us: u64,
}

/// Results for a complete test case
#[derive(Debug)]
struct TestResult {
    test_case: TestCase,
    raw_changeset_bytes: usize,
    cbor: EncodingResult,
    cbor_zstd: EncodingResult,
}

/// Create a test signer
fn test_signer() -> PrivateKeySigner {
    TEST_PRIVATE_KEY.parse().unwrap()
}

/// Get signer's 64-byte uncompressed public key (without 0x04 prefix)
fn signer_pubkey_bytes(signer: &PrivateKeySigner) -> [u8; 64] {
    signer.public_key().0
}

/// Get signer's public key as `VerifyingKey`
fn signer_verifying_key(signer: &PrivateKeySigner) -> VerifyingKey {
    verifying_key_from_bytes(&signer.public_key().0).unwrap()
}

/// Sign data synchronously (returns k256 Signature for COSE)
fn sign_cose(signer: &PrivateKeySigner, data: &[u8]) -> Result<Signature, CborError> {
    let hash = keccak256(data);
    let sig = signer
        .sign_hash_sync(&B256::from(hash))
        .map_err(|e| CborError::Signing(e.to_string()))?;

    let mut bytes = [0u8; 64];
    bytes[..32].copy_from_slice(&sig.r().to_be_bytes::<32>());
    bytes[32..].copy_from_slice(&sig.s().to_be_bytes::<32>());
    signature_from_bytes(&bytes)
}

/// Generate realistic `SQLite` changesets using the session extension
///
/// - `num_changesets`: Number of changesets to generate
/// - `ops_per_changeset`: Number of SQL operations per changeset (simulates activity level)
///   - 1-2 ops: Light activity (single order)
///   - 3-5 ops: Normal trading (order + balance updates)
///   - 10-20 ops: Burst activity (batch of orders, trades)
fn generate_changesets(num_changesets: usize, ops_per_changeset: usize) -> Result<Vec<Vec<u8>>> {
    let conn = Connection::open_in_memory()?;
    initialize_schema(&conn)?;

    // Create users for trading simulation
    for u in 0..10 {
        conn.execute(
            "INSERT INTO users (username) VALUES (?)",
            rusqlite::params![format!("user_{}", u)],
        )?;
    }

    let mut changesets = Vec::with_capacity(num_changesets);
    let mut order_id_counter = 0i64;

    for cs_idx in 0..num_changesets {
        // Create a session to capture changes
        let mut session = Session::new(&conn)?;
        session.attach(None::<&str>)?; // Track all tables

        // Perform ops_per_changeset operations
        for op_idx in 0..ops_per_changeset {
            let global_idx = cs_idx * ops_per_changeset + op_idx;
            let user_id = (global_idx % 10) as i64 + 1;

            // Vary operation types based on index for realistic mix
            match op_idx % 4 {
                0 => {
                    // Insert a new order
                    conn.execute(
                        "INSERT INTO orders (user_id, symbol, side, order_type, price, quantity)
                         VALUES (?, 'BTC-USD', ?, 'limit', ?, ?)",
                        rusqlite::params![
                            user_id,
                            if global_idx.is_multiple_of(2) {
                                "buy"
                            } else {
                                "sell"
                            },
                            50000 + (global_idx as i64 * 10),
                            (global_idx as i64 % 100 + 1) * 10,
                        ],
                    )?;
                    order_id_counter += 1;
                }
                1 => {
                    // Update balance
                    conn.execute(
                        "INSERT OR REPLACE INTO balances (user_id, symbol, amount, locked)
                         VALUES (?, 'BTC-USD', ?, ?)",
                        rusqlite::params![
                            user_id,
                            (global_idx as i64 + 1) * 1000,
                            (global_idx as i64) * 100,
                        ],
                    )?;
                }
                2 => {
                    // Insert ETH balance (different symbol)
                    conn.execute(
                        "INSERT OR REPLACE INTO balances (user_id, symbol, amount, locked)
                         VALUES (?, 'ETH-USD', ?, ?)",
                        rusqlite::params![
                            user_id,
                            (global_idx as i64 + 1) * 500,
                            (global_idx as i64) * 50,
                        ],
                    )?;
                }
                3 => {
                    // Insert another order (different symbol)
                    conn.execute(
                        "INSERT INTO orders (user_id, symbol, side, order_type, price, quantity)
                         VALUES (?, 'ETH-USD', ?, 'limit', ?, ?)",
                        rusqlite::params![
                            user_id,
                            if global_idx.is_multiple_of(2) {
                                "sell"
                            } else {
                                "buy"
                            },
                            3000 + (global_idx as i64 * 5),
                            (global_idx as i64 % 50 + 1) * 5,
                        ],
                    )?;
                    order_id_counter += 1;
                }
                _ => unreachable!(),
            }
        }

        // Get the changeset
        let mut changeset = Vec::new();
        session.changeset_strm(&mut changeset)?;
        changesets.push(changeset);
    }

    // Silence unused variable warning
    let _ = order_id_counter;

    Ok(changesets)
}

/// Combine multiple changesets into a single payload (as synddb-client does)
fn combine_changesets(changesets: &[Vec<u8>]) -> Vec<u8> {
    // Simple concatenation with length prefix for each changeset
    let mut combined = Vec::new();
    for cs in changesets {
        combined.extend_from_slice(&(cs.len() as u32).to_le_bytes());
        combined.extend_from_slice(cs);
    }
    combined
}

/// Create a `CborSignedMessage`
fn create_cbor_message(
    signer: &PrivateKeySigner,
    pubkey: &VerifyingKey,
    sequence: u64,
    timestamp: u64,
    payload: Vec<u8>,
) -> Result<CborSignedMessage> {
    // Note: CBOR format stores payload directly, compression happens at batch level
    let msg = CborSignedMessage::new(
        sequence,
        timestamp,
        CborMessageType::Changeset,
        payload,
        pubkey,
        |data| sign_cose(signer, data),
    )
    .map_err(|e| anyhow::anyhow!("Failed to create CBOR message: {e}"))?;

    Ok(msg)
}

/// Create a `CborBatch`
fn create_cbor_batch(
    signer: &PrivateKeySigner,
    pubkey: [u8; 64],
    messages: Vec<CborSignedMessage>,
    timestamp: u64,
) -> Result<CborBatch> {
    CborBatch::new(messages, timestamp, pubkey, |data| sign_cose(signer, data))
        .map_err(|e| anyhow::anyhow!("Failed to create CBOR batch: {e}"))
}

/// Run benchmark for a single test case
fn run_benchmark(test_case: &TestCase) -> Result<TestResult> {
    let signer = test_signer();
    let pubkey_bytes = signer_pubkey_bytes(&signer);
    let pubkey = signer_verifying_key(&signer);
    let timestamp = 1700000000u64;

    // Generate realistic changesets
    let total_changesets = test_case.num_messages * test_case.changesets_per_message;
    let all_changesets = generate_changesets(total_changesets, test_case.ops_per_changeset)?;

    // Calculate raw changeset bytes
    let raw_changeset_bytes: usize = all_changesets.iter().map(|cs| cs.len()).sum();

    // Group changesets into messages
    let mut message_payloads = Vec::with_capacity(test_case.num_messages);
    for chunk in all_changesets.chunks(test_case.changesets_per_message) {
        message_payloads.push(combine_changesets(chunk));
    }

    // ========================================================================
    // CBOR (uncompressed)
    // ========================================================================
    let cbor_result = {
        let start = Instant::now();

        // Create CBOR messages
        let mut messages = Vec::with_capacity(test_case.num_messages);
        for (i, payload) in message_payloads.iter().enumerate() {
            let msg = create_cbor_message(
                &signer,
                &pubkey,
                i as u64,
                timestamp + i as u64,
                payload.clone(),
            )?;
            messages.push(msg);
        }

        // Create CBOR batch
        let batch = create_cbor_batch(&signer, pubkey_bytes, messages, timestamp)?;

        // Serialize to CBOR (uncompressed)
        let cbor_bytes = batch
            .to_cbor()
            .map_err(|e| anyhow::anyhow!("Failed to serialize CBOR: {e}"))?;

        let encode_time = start.elapsed();

        // Decode timing
        let decode_start = Instant::now();
        let _decoded = CborBatch::from_cbor(&cbor_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to decode CBOR: {e}"))?;
        let decode_time = decode_start.elapsed();

        EncodingResult {
            format_name: "CBOR",
            encoded_bytes: cbor_bytes.len(),
            encode_time_us: encode_time.as_micros() as u64,
            decode_time_us: decode_time.as_micros() as u64,
        }
    };

    // ========================================================================
    // CBOR + zstd (production format)
    // ========================================================================
    let cbor_zstd_result = {
        let start = Instant::now();

        // Create CBOR messages
        let mut messages = Vec::with_capacity(test_case.num_messages);
        for (i, payload) in message_payloads.iter().enumerate() {
            let msg = create_cbor_message(
                &signer,
                &pubkey,
                i as u64,
                timestamp + i as u64,
                payload.clone(),
            )?;
            messages.push(msg);
        }

        // Create CBOR batch
        let batch = create_cbor_batch(&signer, pubkey_bytes, messages, timestamp)?;

        // Serialize to CBOR + zstd
        let cbor_zstd_bytes = batch
            .to_cbor_zstd()
            .map_err(|e| anyhow::anyhow!("Failed to serialize CBOR+zstd: {e}"))?;

        let encode_time = start.elapsed();

        // Decode timing
        let decode_start = Instant::now();
        let _decoded = CborBatch::from_cbor_zstd(&cbor_zstd_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to decode CBOR+zstd: {e}"))?;
        let decode_time = decode_start.elapsed();

        EncodingResult {
            format_name: "CBOR+zstd",
            encoded_bytes: cbor_zstd_bytes.len(),
            encode_time_us: encode_time.as_micros() as u64,
            decode_time_us: decode_time.as_micros() as u64,
        }
    };

    Ok(TestResult {
        test_case: test_case.clone(),
        raw_changeset_bytes,
        cbor: cbor_result,
        cbor_zstd: cbor_zstd_result,
    })
}

/// Print results as a formatted table
fn print_results(results: &[TestResult]) {
    println!();
    println!("================================================================================");
    println!("                       CBOR ENCODING BENCHMARK");
    println!("================================================================================");
    println!();

    for result in results {
        let tc = &result.test_case;
        println!(
            "Test Case: {} (N={} messages, M={} changesets/msg, O={} ops/changeset)",
            tc.name, tc.num_messages, tc.changesets_per_message, tc.ops_per_changeset
        );
        let total_ops = tc.num_messages * tc.changesets_per_message * tc.ops_per_changeset;
        println!(
            "Raw changeset data: {} bytes ({} total SQL operations)",
            result.raw_changeset_bytes, total_ops
        );
        println!();
        println!(
            "  {:<20} {:>12} {:>12} {:>12} {:>12}",
            "Format", "Size (bytes)", "Ratio", "Encode (us)", "Decode (us)"
        );
        println!(
            "  {:-<20} {:->12} {:->12} {:->12} {:->12}",
            "", "", "", "", ""
        );

        let baseline = result.cbor.encoded_bytes as f64;

        for encoding in [&result.cbor, &result.cbor_zstd] {
            let ratio = encoding.encoded_bytes as f64 / baseline;
            println!(
                "  {:<20} {:>12} {:>11.2}x {:>12} {:>12}",
                encoding.format_name,
                encoding.encoded_bytes,
                ratio,
                encoding.encode_time_us,
                encoding.decode_time_us
            );
        }

        // Calculate compression ratio
        let compression_ratio =
            result.cbor.encoded_bytes as f64 / result.cbor_zstd.encoded_bytes as f64;
        println!();
        println!(
            "  Compression ratio: {:.2}x (zstd reduces size by {:.1}%)",
            compression_ratio,
            (1.0 - 1.0 / compression_ratio) * 100.0
        );
        println!();
        println!(
            "--------------------------------------------------------------------------------"
        );
        println!();
    }

    // Summary
    println!("SUMMARY");
    println!("=======");
    println!();
    println!("Format comparison across all test cases:");
    println!();

    let mut total_cbor: usize = 0;
    let mut total_cbor_zstd: usize = 0;

    for result in results {
        total_cbor += result.cbor.encoded_bytes;
        total_cbor_zstd += result.cbor_zstd.encoded_bytes;
    }

    println!("  CBOR total:      {} bytes", total_cbor);
    println!(
        "  CBOR+zstd total: {} bytes ({:.2}x compression)",
        total_cbor_zstd,
        total_cbor as f64 / total_cbor_zstd as f64
    );
    println!();
}

#[test]
fn test_encoding_benchmark() {
    let test_cases = vec![
        // Light activity scenarios (1-2 ops per changeset)
        TestCase {
            name: "quiet_minimal",
            num_messages: 2,
            changesets_per_message: 1,
            ops_per_changeset: 1,
        },
        TestCase {
            name: "quiet_light",
            num_messages: 2,
            changesets_per_message: 2,
            ops_per_changeset: 2,
        },
        // Normal trading scenarios (3-5 ops per changeset)
        TestCase {
            name: "normal_small",
            num_messages: 5,
            changesets_per_message: 4,
            ops_per_changeset: 3,
        },
        TestCase {
            name: "normal_medium",
            num_messages: 5,
            changesets_per_message: 5,
            ops_per_changeset: 4,
        },
        TestCase {
            name: "normal_large",
            num_messages: 10,
            changesets_per_message: 10,
            ops_per_changeset: 5,
        },
        // Burst activity scenarios (10-20 ops per changeset)
        TestCase {
            name: "burst_small",
            num_messages: 5,
            changesets_per_message: 5,
            ops_per_changeset: 10,
        },
        TestCase {
            name: "burst_medium",
            num_messages: 10,
            changesets_per_message: 5,
            ops_per_changeset: 15,
        },
        TestCase {
            name: "burst_large",
            num_messages: 10,
            changesets_per_message: 10,
            ops_per_changeset: 20,
        },
    ];

    let mut results = Vec::with_capacity(test_cases.len());

    for tc in &test_cases {
        match run_benchmark(tc) {
            Ok(result) => results.push(result),
            Err(e) => {
                eprintln!("Failed to run benchmark for {}: {}", tc.name, e);
                panic!("Benchmark failed");
            }
        }
    }

    print_results(&results);

    // Assert that CBOR+zstd is always smaller than uncompressed CBOR
    for result in &results {
        assert!(
            result.cbor_zstd.encoded_bytes <= result.cbor.encoded_bytes,
            "CBOR+zstd ({}) should be <= CBOR ({}) for test case {}",
            result.cbor_zstd.encoded_bytes,
            result.cbor.encoded_bytes,
            result.test_case.name
        );
    }
}
