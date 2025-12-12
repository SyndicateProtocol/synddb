//! Integration tests for CBOR/COSE types with real signing

use alloy::{
    primitives::{keccak256, B256},
    signers::{local::PrivateKeySigner, SignerSync},
};
use synddb_shared::types::{
    cbor::{
        batch::CborBatch,
        error::CborError,
        message::{CborMessageType, CborSignedMessage},
    },
    message::SignedBatch,
};

/// Test private key (well-known test key, do not use in production)
const TEST_PRIVATE_KEY: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

/// Create a test signer
fn test_signer() -> PrivateKeySigner {
    TEST_PRIVATE_KEY.parse().unwrap()
}

/// Get signer address as bytes
const fn signer_address(signer: &PrivateKeySigner) -> [u8; 20] {
    signer.address().into_array()
}

/// Sign a message synchronously (returns 64-byte signature)
fn sign_sync(signer: &PrivateKeySigner, data: &[u8]) -> Result<[u8; 64], CborError> {
    let hash = keccak256(data);
    let sig = signer
        .sign_hash_sync(&B256::from(hash))
        .map_err(|e| CborError::Signing(e.to_string()))?;

    // Extract r and s (64 bytes total, no v)
    let mut result = [0u8; 64];
    result[..32].copy_from_slice(&sig.r().to_be_bytes::<32>());
    result[32..].copy_from_slice(&sig.s().to_be_bytes::<32>());
    Ok(result)
}

#[test]
fn test_signed_message_roundtrip() {
    let signer = test_signer();
    let addr = signer_address(&signer);

    let payload = b"test payload data".to_vec();
    let sequence = 42u64;
    let timestamp = 1700000000u64;

    // Create signed message
    let msg = CborSignedMessage::new(
        sequence,
        timestamp,
        CborMessageType::Changeset,
        payload.clone(),
        addr,
        |data| sign_sync(&signer, data),
    )
    .unwrap();

    // Verify we can extract sequence
    assert_eq!(msg.sequence().unwrap(), sequence);

    // Verify and parse
    let parsed = msg.verify_and_parse(&addr).unwrap();
    assert_eq!(parsed.sequence, sequence);
    assert_eq!(parsed.timestamp, timestamp);
    assert_eq!(parsed.message_type, CborMessageType::Changeset);
    assert_eq!(parsed.payload, payload);
    assert_eq!(parsed.signer, addr);
}

#[test]
fn test_signed_message_wrong_signer_fails() {
    let signer = test_signer();
    let addr = signer_address(&signer);
    let wrong_addr = [0u8; 20]; // All zeros

    let msg = CborSignedMessage::new(
        1,
        1700000000,
        CborMessageType::Changeset,
        b"test".to_vec(),
        addr,
        |data| sign_sync(&signer, data),
    )
    .unwrap();

    // Should fail with wrong signer
    let result = msg.verify_and_parse(&wrong_addr);
    assert!(result.is_err());
    assert!(matches!(result, Err(CborError::SignatureVerification(_))));
}

#[test]
fn test_batch_roundtrip() {
    let signer = test_signer();
    let addr = signer_address(&signer);

    // Create multiple messages
    let mut messages = Vec::new();
    for i in 0..3 {
        let msg = CborSignedMessage::new(
            i + 1,
            1700000000 + i,
            CborMessageType::Changeset,
            format!("payload {}", i).into_bytes(),
            addr,
            |data| sign_sync(&signer, data),
        )
        .unwrap();
        messages.push(msg);
    }

    // Create batch
    let batch =
        CborBatch::new(messages, 1700000000, addr, |data| sign_sync(&signer, data)).unwrap();

    assert_eq!(batch.start_sequence, 1);
    assert_eq!(batch.end_sequence, 3);
    assert_eq!(batch.message_count(), 3);
    assert_eq!(batch.signer, addr);

    // Serialize to CBOR
    let cbor = batch.to_cbor().unwrap();

    // Parse back
    let parsed = CborBatch::from_cbor(&cbor).unwrap();
    assert_eq!(parsed.start_sequence, batch.start_sequence);
    assert_eq!(parsed.end_sequence, batch.end_sequence);
    assert_eq!(parsed.message_count(), batch.message_count());
    assert_eq!(parsed.content_hash, batch.content_hash);

    // Verify all signatures
    parsed.verify_all_signatures().unwrap();
}

#[test]
fn test_batch_zstd_roundtrip() {
    let signer = test_signer();
    let addr = signer_address(&signer);

    let msg = CborSignedMessage::new(
        1,
        1700000000,
        CborMessageType::Changeset,
        b"test payload".to_vec(),
        addr,
        |data| sign_sync(&signer, data),
    )
    .unwrap();

    let batch =
        CborBatch::new(vec![msg], 1700000000, addr, |data| sign_sync(&signer, data)).unwrap();

    // Serialize with compression
    let compressed = batch.to_cbor_zstd().unwrap();
    let uncompressed = batch.to_cbor().unwrap();

    // Compressed should be smaller (or similar for small data)
    println!(
        "Uncompressed: {} bytes, Compressed: {} bytes",
        uncompressed.len(),
        compressed.len()
    );

    // Parse from compressed
    let parsed = CborBatch::from_cbor_zstd(&compressed).unwrap();
    assert_eq!(parsed.start_sequence, batch.start_sequence);
    parsed.verify_all_signatures().unwrap();
}

#[test]
fn test_batch_to_json() {
    let signer = test_signer();
    let addr = signer_address(&signer);

    let msg = CborSignedMessage::new(
        42,
        1700000000,
        CborMessageType::Changeset,
        b"test payload".to_vec(),
        addr,
        |data| sign_sync(&signer, data),
    )
    .unwrap();

    let batch =
        CborBatch::new(vec![msg], 1700000000, addr, |data| sign_sync(&signer, data)).unwrap();

    // Convert to JSON
    let json = batch.to_json_pretty().unwrap();
    println!("Batch JSON:\n{}", json);

    // Parse JSON to verify structure
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["version"], 1);
    assert_eq!(value["start_sequence"], 42);
    assert_eq!(value["end_sequence"], 42);
    assert_eq!(value["message_count"], 1);
    assert!(value["content_hash"].as_str().unwrap().starts_with("0x"));
    assert!(value["batch_signature"].as_str().unwrap().starts_with("0x"));
    assert!(value["signer"].as_str().unwrap().starts_with("0x"));
}

#[test]
fn test_message_to_json() {
    let signer = test_signer();
    let addr = signer_address(&signer);

    let msg = CborSignedMessage::new(
        42,
        1700000000,
        CborMessageType::Withdrawal,
        b"withdrawal data".to_vec(),
        addr,
        |data| sign_sync(&signer, data),
    )
    .unwrap();

    let json = msg.to_json_pretty().unwrap();
    println!("Message JSON:\n{}", json);

    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["sequence"], 42);
    assert_eq!(value["timestamp"], 1700000000u64);
    assert_eq!(value["message_type"], "Withdrawal");
}

#[test]
fn test_content_hash_determinism() {
    let signer = test_signer();
    let addr = signer_address(&signer);

    // Create same messages twice
    let create_messages = || {
        (0..3)
            .map(|i| {
                CborSignedMessage::new(
                    i + 1,
                    1700000000,
                    CborMessageType::Changeset,
                    format!("payload {}", i).into_bytes(),
                    addr,
                    |data| sign_sync(&signer, data),
                )
                .unwrap()
            })
            .collect::<Vec<_>>()
    };

    let batch1 = CborBatch::new(create_messages(), 1700000000, addr, |data| {
        sign_sync(&signer, data)
    })
    .unwrap();

    let batch2 = CborBatch::new(create_messages(), 1700000000, addr, |data| {
        sign_sync(&signer, data)
    })
    .unwrap();

    // Content hashes should be equal (same messages)
    assert_eq!(batch1.content_hash, batch2.content_hash);
}

#[test]
fn test_empty_batch_fails() {
    let signer = test_signer();
    let addr = signer_address(&signer);

    let result = CborBatch::new(vec![], 1700000000, addr, |data| sign_sync(&signer, data));

    assert!(result.is_err());
    assert!(matches!(result, Err(CborError::InvalidBatch(_))));
}

#[test]
fn test_all_message_types() {
    let signer = test_signer();
    let addr = signer_address(&signer);

    for msg_type in [
        CborMessageType::Changeset,
        CborMessageType::Withdrawal,
        CborMessageType::Snapshot,
    ] {
        let msg = CborSignedMessage::new(1, 1700000000, msg_type, b"test".to_vec(), addr, |data| {
            sign_sync(&signer, data)
        })
        .unwrap();

        let parsed = msg.verify_and_parse(&addr).unwrap();
        assert_eq!(parsed.message_type, msg_type);
    }
}

#[test]
fn test_large_payload() {
    let signer = test_signer();
    let addr = signer_address(&signer);

    // 100KB payload
    let payload = vec![0xABu8; 100 * 1024];

    let msg = CborSignedMessage::new(
        1,
        1700000000,
        CborMessageType::Snapshot,
        payload.clone(),
        addr,
        |data| sign_sync(&signer, data),
    )
    .unwrap();

    let batch =
        CborBatch::new(vec![msg], 1700000000, addr, |data| sign_sync(&signer, data)).unwrap();

    // Check compression ratio
    let uncompressed = batch.to_cbor().unwrap();
    let compressed = batch.to_cbor_zstd().unwrap();

    println!(
        "Large payload - Uncompressed: {} bytes, Compressed: {} bytes, Ratio: {:.2}x",
        uncompressed.len(),
        compressed.len(),
        uncompressed.len() as f64 / compressed.len() as f64
    );

    // Verify roundtrip
    let parsed = CborBatch::from_cbor_zstd(&compressed).unwrap();
    parsed.verify_all_signatures().unwrap();

    let msg_parsed = parsed.messages[0].verify_and_parse(&addr).unwrap();
    assert_eq!(msg_parsed.payload, payload);
}

#[test]
fn test_batch_size_tracking() {
    let signer = test_signer();
    let addr = signer_address(&signer);

    let mut messages = Vec::new();
    for i in 0..10 {
        let msg = CborSignedMessage::new(
            i + 1,
            1700000000,
            CborMessageType::Changeset,
            vec![0u8; 1000], // 1KB payload each
            addr,
            |data| sign_sync(&signer, data),
        )
        .unwrap();
        messages.push(msg);
    }

    let batch =
        CborBatch::new(messages, 1700000000, addr, |data| sign_sync(&signer, data)).unwrap();

    // Total message bytes should be tracked
    let total_bytes = batch.total_message_bytes();
    println!("Total message bytes: {}", total_bytes);
    assert!(total_bytes > 10 * 1000); // At least 10KB of payloads
}

/// Test that `CborBatch` -> `SignedBatch` conversion preserves verification capability
#[test]
fn test_cbor_to_signed_batch_verification() {
    let signer = test_signer();
    let addr = signer_address(&signer);

    // Create a CBOR message
    let msg = CborSignedMessage::new(
        1,
        1700000000,
        CborMessageType::Changeset,
        b"test payload".to_vec(),
        addr,
        |data| sign_sync(&signer, data),
    )
    .unwrap();

    // Create a CBOR batch
    let cbor_batch =
        CborBatch::new(vec![msg], 1700000000, addr, |data| sign_sync(&signer, data)).unwrap();

    // Verify CBOR batch signature
    cbor_batch.verify_batch_signature().unwrap();

    // Convert to SignedBatch (JSON format)
    let signed_batch: SignedBatch = cbor_batch.to_signed_batch().unwrap();

    // Verify cbor_content_hash is set
    assert!(
        signed_batch.cbor_content_hash.is_some(),
        "cbor_content_hash should be set after conversion"
    );
    assert_eq!(
        signed_batch.cbor_content_hash.unwrap(),
        cbor_batch.content_hash
    );

    // The SignedBatch should now be verifiable
    signed_batch.verify_batch_signature().unwrap();
}

/// Test that `SignedBatch` converted from CBOR survives JSON serialization
#[test]
fn test_cbor_to_signed_batch_json_roundtrip_verification() {
    let signer = test_signer();
    let addr = signer_address(&signer);

    // Create a CBOR batch
    let msg = CborSignedMessage::new(
        42,
        1700000000,
        CborMessageType::Changeset,
        b"test payload".to_vec(),
        addr,
        |data| sign_sync(&signer, data),
    )
    .unwrap();

    let cbor_batch =
        CborBatch::new(vec![msg], 1700000000, addr, |data| sign_sync(&signer, data)).unwrap();

    // Convert to SignedBatch
    let signed_batch = cbor_batch.to_signed_batch().unwrap();

    // Serialize to JSON and back (simulating HTTP transport)
    let json = serde_json::to_string(&signed_batch).unwrap();
    println!("SignedBatch JSON length: {} bytes", json.len());

    let deserialized: SignedBatch = serde_json::from_str(&json).unwrap();

    // Verify cbor_content_hash survived serialization
    assert!(
        deserialized.cbor_content_hash.is_some(),
        "cbor_content_hash should survive JSON roundtrip"
    );
    assert_eq!(
        deserialized.cbor_content_hash,
        signed_batch.cbor_content_hash
    );

    // The deserialized batch should still be verifiable
    deserialized.verify_batch_signature().unwrap();
}
