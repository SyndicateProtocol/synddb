//! Encoding format comparison benchmark
//!
//! Compares file sizes and decode times for different encoding/compression combinations:
//! - JSON + base64 + zstd (legacy format)
//! - CBOR (uncompressed)
//! - CBOR + zstd (current production format)

use alloy::{
    primitives::{keccak256, B256},
    signers::{local::PrivateKeySigner, SignerSync},
};
use anyhow::Result;
use rusqlite::{session::Session, Connection};
use std::{io::Write, time::Instant};
use synddb_benchmark::schema::initialize_schema;
use synddb_shared::types::{
    cbor::{
        batch::CborBatch,
        error::CborError,
        message::{CborMessageType, CborSignedMessage},
    },
    message::{MessageType, SignedBatch, SignedMessage},
};

/// Test private key (well-known test key, do not use in production)
const TEST_PRIVATE_KEY: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

/// Test case configuration
#[derive(Debug, Clone)]
struct TestCase {
    name: &'static str,
    num_messages: usize,
    changesets_per_message: usize,
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
    json_base64_zstd: EncodingResult,
    cbor: EncodingResult,
    cbor_zstd: EncodingResult,
}

/// Create a test signer
fn test_signer() -> PrivateKeySigner {
    TEST_PRIVATE_KEY.parse().unwrap()
}

/// Get signer address as bytes
const fn signer_address(signer: &PrivateKeySigner) -> [u8; 20] {
    signer.address().into_array()
}

/// Sign data synchronously (returns 64-byte signature for COSE)
fn sign_cose(signer: &PrivateKeySigner, data: &[u8]) -> Result<[u8; 64], CborError> {
    let hash = keccak256(data);
    let sig = signer
        .sign_hash_sync(&B256::from(hash))
        .map_err(|e| CborError::Signing(e.to_string()))?;

    let mut result = [0u8; 64];
    result[..32].copy_from_slice(&sig.r().to_be_bytes::<32>());
    result[32..].copy_from_slice(&sig.s().to_be_bytes::<32>());
    Ok(result)
}

/// Sign data synchronously (returns 65-byte signature for legacy format)
fn sign_legacy(signer: &PrivateKeySigner, data: B256) -> Result<[u8; 65]> {
    let sig = signer
        .sign_hash_sync(&data)
        .map_err(|e| anyhow::anyhow!("Signing failed: {e}"))?;

    let mut result = [0u8; 65];
    result[..32].copy_from_slice(&sig.r().to_be_bytes::<32>());
    result[32..64].copy_from_slice(&sig.s().to_be_bytes::<32>());
    result[64] = if sig.v() { 28 } else { 27 };
    Ok(result)
}

/// Generate realistic `SQLite` changesets using the session extension
fn generate_changesets(num_changesets: usize) -> Result<Vec<Vec<u8>>> {
    let conn = Connection::open_in_memory()?;
    initialize_schema(&conn)?;

    // Create a user first
    conn.execute("INSERT INTO users (username) VALUES ('benchmark_user')", [])?;

    let mut changesets = Vec::with_capacity(num_changesets);

    for i in 0..num_changesets {
        // Create a session to capture changes
        let mut session = Session::new(&conn)?;
        session.attach(None::<&str>)?; // Track all tables

        // Insert an order (realistic orderbook operation)
        conn.execute(
            "INSERT INTO orders (user_id, symbol, side, order_type, price, quantity)
             VALUES (1, 'BTC-USD', ?, 'limit', ?, ?)",
            rusqlite::params![
                if i % 2 == 0 { "buy" } else { "sell" },
                50000 + (i as i64 * 100),
                (i as i64 + 1) * 10,
            ],
        )?;

        // Update a balance (another common operation)
        conn.execute(
            "INSERT OR REPLACE INTO balances (user_id, symbol, amount, locked)
             VALUES (1, 'BTC-USD', ?, ?)",
            rusqlite::params![(i as i64 + 1) * 1000, (i as i64) * 100,],
        )?;

        // Get the changeset
        let mut changeset = Vec::new();
        session.changeset_strm(&mut changeset)?;
        changesets.push(changeset);
    }

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

/// Compress payload with zstd
fn zstd_compress(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = zstd::Encoder::new(Vec::new(), 3)?;
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

/// Decompress zstd data
fn zstd_decompress(data: &[u8]) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut decoder = zstd::Decoder::new(data)?;
    let mut result = Vec::new();
    decoder.read_to_end(&mut result)?;
    Ok(result)
}

/// Create a `SignedMessage` in legacy JSON format
fn create_legacy_message(
    signer: &PrivateKeySigner,
    sequence: u64,
    timestamp: u64,
    payload: Vec<u8>,
) -> Result<SignedMessage> {
    // Compress payload with zstd (as done in production)
    let compressed_payload = zstd_compress(&payload)?;

    // Compute message hash
    let message_hash = keccak256(&compressed_payload);

    // Compute signing payload
    let signing_payload = SignedMessage::compute_signing_payload(sequence, timestamp, message_hash);

    // Sign
    let signature = sign_legacy(signer, signing_payload)?;

    Ok(SignedMessage {
        sequence,
        timestamp,
        message_type: MessageType::Changeset,
        payload: compressed_payload,
        message_hash: format!("0x{}", hex::encode(message_hash)),
        signature: format!("0x{}", hex::encode(signature)),
        signer: format!("{:?}", signer.address()),
        cose_protected_header: None,
    })
}

/// Create a `SignedBatch` in legacy JSON format
fn create_legacy_batch(
    signer: &PrivateKeySigner,
    messages: Vec<SignedMessage>,
) -> Result<SignedBatch> {
    let start_sequence = messages.first().map_or(0, |m| m.sequence);
    let end_sequence = messages.last().map_or(0, |m| m.sequence);
    let timestamp = messages.first().map_or(0, |m| m.timestamp);

    // Compute messages hash
    let messages_hash = SignedBatch::compute_messages_hash(&messages)
        .map_err(|e| anyhow::anyhow!("Failed to compute messages hash: {e}"))?;

    // Compute signing payload
    let signing_payload =
        SignedBatch::compute_signing_payload(start_sequence, end_sequence, messages_hash);

    // Sign
    let signature = sign_legacy(signer, signing_payload)?;

    Ok(SignedBatch {
        start_sequence,
        end_sequence,
        messages,
        batch_signature: format!("0x{}", hex::encode(signature)),
        signer: format!("{:?}", signer.address()),
        created_at: timestamp,
        cbor_content_hash: None,
    })
}

/// Create a `CborSignedMessage`
fn create_cbor_message(
    signer: &PrivateKeySigner,
    addr: [u8; 20],
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
        addr,
        |data| sign_cose(signer, data),
    )
    .map_err(|e| anyhow::anyhow!("Failed to create CBOR message: {e}"))?;

    Ok(msg)
}

/// Create a `CborBatch`
fn create_cbor_batch(
    signer: &PrivateKeySigner,
    addr: [u8; 20],
    messages: Vec<CborSignedMessage>,
    timestamp: u64,
) -> Result<CborBatch> {
    CborBatch::new(messages, timestamp, addr, |data| sign_cose(signer, data))
        .map_err(|e| anyhow::anyhow!("Failed to create CBOR batch: {e}"))
}

/// Run benchmark for a single test case
fn run_benchmark(test_case: &TestCase) -> Result<TestResult> {
    let signer = test_signer();
    let addr = signer_address(&signer);
    let timestamp = 1700000000u64;

    // Generate realistic changesets
    let total_changesets = test_case.num_messages * test_case.changesets_per_message;
    let all_changesets = generate_changesets(total_changesets)?;

    // Calculate raw changeset bytes
    let raw_changeset_bytes: usize = all_changesets.iter().map(|cs| cs.len()).sum();

    // Group changesets into messages
    let mut message_payloads = Vec::with_capacity(test_case.num_messages);
    for chunk in all_changesets.chunks(test_case.changesets_per_message) {
        message_payloads.push(combine_changesets(chunk));
    }

    // ========================================================================
    // JSON + base64 + zstd (legacy format)
    // ========================================================================
    let json_result = {
        let start = Instant::now();

        // Create legacy messages
        let mut messages = Vec::with_capacity(test_case.num_messages);
        for (i, payload) in message_payloads.iter().enumerate() {
            let msg =
                create_legacy_message(&signer, i as u64, timestamp + i as u64, payload.clone())?;
            messages.push(msg);
        }

        // Create legacy batch
        let batch = create_legacy_batch(&signer, messages)?;

        // Serialize to JSON (payload is already base64 encoded by serde)
        let json_bytes = serde_json::to_vec(&batch)?;

        // Compress with zstd
        let compressed = zstd_compress(&json_bytes)?;

        let encode_time = start.elapsed();

        // Decode timing
        let decode_start = Instant::now();
        let decompressed = zstd_decompress(&compressed)?;
        let _decoded: SignedBatch = serde_json::from_slice(&decompressed)?;
        let decode_time = decode_start.elapsed();

        EncodingResult {
            format_name: "JSON+base64+zstd",
            encoded_bytes: compressed.len(),
            encode_time_us: encode_time.as_micros() as u64,
            decode_time_us: decode_time.as_micros() as u64,
        }
    };

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
                addr,
                i as u64,
                timestamp + i as u64,
                payload.clone(),
            )?;
            messages.push(msg);
        }

        // Create CBOR batch
        let batch = create_cbor_batch(&signer, addr, messages, timestamp)?;

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
                addr,
                i as u64,
                timestamp + i as u64,
                payload.clone(),
            )?;
            messages.push(msg);
        }

        // Create CBOR batch
        let batch = create_cbor_batch(&signer, addr, messages, timestamp)?;

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
        json_base64_zstd: json_result,
        cbor: cbor_result,
        cbor_zstd: cbor_zstd_result,
    })
}

/// Print results as a formatted table
fn print_results(results: &[TestResult]) {
    println!();
    println!("================================================================================");
    println!("                    ENCODING FORMAT COMPARISON BENCHMARK");
    println!("================================================================================");
    println!();

    for result in results {
        let tc = &result.test_case;
        println!(
            "Test Case: {} (N={} messages, M={} changesets/message)",
            tc.name, tc.num_messages, tc.changesets_per_message
        );
        println!("Raw changeset data: {} bytes", result.raw_changeset_bytes);
        println!();
        println!(
            "  {:<20} {:>12} {:>12} {:>12} {:>12}",
            "Format", "Size (bytes)", "Ratio", "Encode (us)", "Decode (us)"
        );
        println!(
            "  {:-<20} {:->12} {:->12} {:->12} {:->12}",
            "", "", "", "", ""
        );

        let baseline = result.json_base64_zstd.encoded_bytes as f64;

        for encoding in [&result.json_base64_zstd, &result.cbor, &result.cbor_zstd] {
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

        // Calculate improvement
        let best_size = result.cbor_zstd.encoded_bytes;
        let improvement = (1.0 - (best_size as f64 / baseline)) * 100.0;
        println!();
        println!(
            "  CBOR+zstd improvement over JSON+base64+zstd: {:.1}% smaller",
            improvement
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

    let mut total_json: usize = 0;
    let mut total_cbor: usize = 0;
    let mut total_cbor_zstd: usize = 0;

    for result in results {
        total_json += result.json_base64_zstd.encoded_bytes;
        total_cbor += result.cbor.encoded_bytes;
        total_cbor_zstd += result.cbor_zstd.encoded_bytes;
    }

    println!("  JSON+base64+zstd total: {} bytes", total_json);
    println!(
        "  CBOR total:             {} bytes ({:.1}x vs JSON)",
        total_cbor,
        total_cbor as f64 / total_json as f64
    );
    println!(
        "  CBOR+zstd total:        {} bytes ({:.1}x vs JSON)",
        total_cbor_zstd,
        total_cbor_zstd as f64 / total_json as f64
    );
    println!();

    let overall_improvement = (1.0 - (total_cbor_zstd as f64 / total_json as f64)) * 100.0;
    println!(
        "Overall CBOR+zstd improvement: {:.1}% smaller than JSON+base64+zstd",
        overall_improvement
    );
    println!();
}

#[test]
fn test_encoding_benchmark() {
    let test_cases = vec![
        TestCase {
            name: "small_balanced",
            num_messages: 5,
            changesets_per_message: 5,
        },
        TestCase {
            name: "medium_balanced",
            num_messages: 10,
            changesets_per_message: 10,
        },
        TestCase {
            name: "few_single",
            num_messages: 2,
            changesets_per_message: 1,
        },
        TestCase {
            name: "few_multiple",
            num_messages: 2,
            changesets_per_message: 4,
        },
        TestCase {
            name: "medium_multiple",
            num_messages: 5,
            changesets_per_message: 4,
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

    // Assert that CBOR+zstd is always the smallest
    for result in &results {
        assert!(
            result.cbor_zstd.encoded_bytes <= result.json_base64_zstd.encoded_bytes,
            "CBOR+zstd ({}) should be <= JSON+base64+zstd ({}) for test case {}",
            result.cbor_zstd.encoded_bytes,
            result.json_base64_zstd.encoded_bytes,
            result.test_case.name
        );
    }
}
