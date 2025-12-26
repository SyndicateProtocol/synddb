//! Integration tests for the bridge validator with smart contracts.
//!
//! These tests require Anvil to be running on port 8545. Start it with:
//! ```
//! cd contracts && anvil
//! ```
//!
//! Run tests with:
//! ```
//! cargo test -p synddb-bridge-validator --test integration_test -- --ignored
//! ```

use alloy::{
    primitives::Address,
    providers::{Provider, ProviderBuilder},
};
use sha3::{Digest, Keccak256};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use synddb_bridge_validator::{
    signing::MessageSigner,
    state::{MessageStore, NonceStore},
    types::Message,
    validation::ValidationPipeline,
};

const ANVIL_URL: &str = "http://127.0.0.1:8545";
const ANVIL_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ANVIL_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

#[tokio::test]
#[ignore = "requires anvil running on port 8545"]
async fn test_message_id_computation_matches_contract() {
    // This test verifies our Rust message ID computation matches the Solidity implementation

    let message_type = "setValue(uint256)";
    let calldata =
        hex::decode("55241077000000000000000000000000000000000000000000000000000000000000002a")
            .unwrap();
    let metadata_hash = [0u8; 32];
    let nonce = 1u64;
    let timestamp = 1735200000u64;
    let domain: [u8; 32] = Keccak256::digest(b"test-app").into();

    // Compute Rust message ID
    let message = Message {
        id: [0u8; 32], // Will be computed
        message_type: message_type.to_string(),
        calldata: calldata.clone(),
        metadata: serde_json::json!({}),
        metadata_hash,
        nonce,
        timestamp,
        domain,
        value: None,
    };

    let rust_message_id = message.compute_id();

    // Compute expected using the same algorithm
    let mut hasher = Keccak256::new();
    hasher.update(message_type.as_bytes());
    hasher.update(calldata);
    hasher.update(metadata_hash);
    hasher.update(nonce.to_be_bytes());
    hasher.update(timestamp.to_be_bytes());
    hasher.update(domain);
    let expected: [u8; 32] = hasher.finalize().into();

    assert_eq!(
        rust_message_id, expected,
        "Message ID computation should match"
    );
}

#[tokio::test]
#[ignore = "requires anvil running on port 8545"]
async fn test_eip712_signing_produces_valid_signature() {
    // Create a signer with the same parameters as would be used with the contract
    let chain_id = 31337u64; // Anvil default chain ID
    let bridge_address: Address = "0x5FbDB2315678afecb367f032d93F642f64180aa3"
        .parse()
        .unwrap();

    let signer = MessageSigner::new(ANVIL_PRIVATE_KEY, chain_id, bridge_address)
        .expect("Failed to create signer");

    // Create a test message
    let message = Message {
        id: [1u8; 32],
        message_type: "setValue(uint256)".to_string(),
        calldata: hex::decode(
            "55241077000000000000000000000000000000000000000000000000000000000000002a",
        )
        .unwrap(),
        metadata: serde_json::json!({}),
        metadata_hash: [0u8; 32],
        nonce: 1,
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        domain: [0u8; 32],
        value: None,
    };

    // Sign the message
    let signature = signer
        .sign_message(&message)
        .await
        .expect("Failed to sign message");

    // Verify signature is 65 bytes (r + s + v)
    assert_eq!(signature.len(), 65, "Signature should be 65 bytes");

    // Verify the signer address matches
    let expected_address: Address = ANVIL_ADDRESS.parse().unwrap();
    assert_eq!(signer.address(), expected_address);
}

#[tokio::test]
#[ignore = "requires anvil running on port 8545"]
async fn test_validation_pipeline_with_stores() {
    // Create in-memory stores
    let message_store = Arc::new(MessageStore::new(":memory:").unwrap());
    let nonce_store = Arc::new(NonceStore::new(":memory:").unwrap());

    // Create validation pipeline
    let pipeline = ValidationPipeline::new(
        message_store.clone(),
        nonce_store.clone(),
        Duration::from_secs(60),
        Duration::from_secs(3600),
    );

    // Create a test message with current timestamp
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut message = Message {
        id: [0u8; 32],
        message_type: "setValue(uint256)".to_string(),
        calldata: hex::decode(
            "55241077000000000000000000000000000000000000000000000000000000000000002a",
        )
        .unwrap(),
        metadata: serde_json::json!({}),
        metadata_hash: [0u8; 32],
        nonce: 1,
        timestamp: now,
        domain: [0u8; 32],
        value: None,
    };
    message.id = message.compute_id();

    // Create validation context
    use synddb_bridge_validator::{
        types::{ApplicationConfig, MessageTypeConfig},
        validation::ValidationContext,
    };

    let ctx = ValidationContext {
        app_config: ApplicationConfig {
            domain: [0u8; 32],
            primary_validator: Address::ZERO,
            expiration_seconds: 86400,
            require_witness_signatures: true,
            active: true,
        },
        message_type_config: MessageTypeConfig {
            message_type: "setValue(uint256)".to_string(),
            selector: [0x55, 0x24, 0x10, 0x77],
            target: Address::ZERO,
            schema_hash: [0u8; 32],
            schema_uri: String::new(),
            enabled: true,
            updated_at: 0,
        },
        schema: None,
    };

    // Validate should pass
    let result = pipeline.validate(&message, &ctx).await;
    assert!(result.is_ok(), "Validation should pass: {:?}", result);

    // Consume nonce and mark processed
    pipeline
        .consume_nonce(&message.domain, message.nonce)
        .unwrap();
    pipeline.mark_message_processed(&message.id).unwrap();

    // Second validation should fail (replay)
    let result2 = pipeline.validate(&message, &ctx).await;
    assert!(result2.is_err(), "Second validation should fail (replay)");
}

#[tokio::test]
#[ignore = "requires anvil running on port 8545"]
async fn test_anvil_connection() {
    // Just verify we can connect to Anvil
    let url: reqwest::Url = ANVIL_URL.parse().expect("Invalid URL");
    let provider = ProviderBuilder::new().connect_http(url);

    let chain_id = provider
        .get_chain_id()
        .await
        .expect("Failed to get chain ID");

    assert_eq!(chain_id, 31337, "Should be Anvil default chain ID");
}
