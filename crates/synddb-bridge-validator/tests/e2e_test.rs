//! End-to-end tests for the bridge validator against real contracts.
//!
//! These tests require Anvil to be running with the MessageBridge contract deployed.
//! Run the setup script first:
//! ```
//! ./scripts/setup-e2e-test.sh
//! ```
//!
//! Then run tests with:
//! ```
//! cargo test -p synddb-bridge-validator --test e2e_test -- --ignored
//! ```

use alloy::primitives::{Address, FixedBytes};
use alloy::providers::ProviderBuilder;
use alloy::sol;
use sha3::{Digest, Keccak256};
use std::time::{SystemTime, UNIX_EPOCH};

use synddb_bridge_validator::bridge::BridgeClient;
use synddb_bridge_validator::signing::{compute_domain_separator, compute_message_id, MessageSigner};
use synddb_bridge_validator::types::Message;

const ANVIL_URL: &str = "http://127.0.0.1:8545";
const ANVIL_CHAIN_ID: u64 = 31337;

// Anvil default accounts (deterministic)
const ADMIN_PRIVATE_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const VALIDATOR_PRIVATE_KEY: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
const WITNESS_PRIVATE_KEY: &str = "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a";

sol! {
    #[sol(rpc)]
    interface IMessageBridge {
        function computeMessageId(
            string calldata messageType,
            bytes calldata calldata_,
            bytes32 metadataHash,
            uint64 nonce,
            uint64 timestamp,
            bytes32 domain
        ) external pure returns (bytes32);

        function getLastNonce(bytes32 domain) external view returns (uint64);
        function getMessageStage(bytes32 messageId) external view returns (uint8);
        function getSignatureCount(bytes32 messageId) external view returns (uint256);
        function hasValidatorSigned(bytes32 messageId, address validator) external view returns (bool);
        function signatureThreshold() external view returns (uint256);
        function DOMAIN_SEPARATOR() external view returns (bytes32);

        function signMessage(bytes32 messageId, bytes calldata signature) external;
    }
}

async fn get_bridge_address() -> Option<Address> {
    // Read from environment or use default deployment address
    std::env::var("BRIDGE_ADDRESS")
        .ok()
        .and_then(|s| s.parse().ok())
}

#[tokio::test]
#[ignore = "requires anvil with deployed contracts"]
async fn test_e2e_message_id_computation_matches_contract() {
    let bridge_address = get_bridge_address()
        .await
        .expect("BRIDGE_ADDRESS not set. Run setup-e2e-test.sh first.");

    let url: reqwest::Url = ANVIL_URL.parse().unwrap();
    let provider = ProviderBuilder::new().connect_http(url);
    let contract = IMessageBridge::new(bridge_address, &provider);

    // Test parameters
    let message_type = "setValue(uint256)";
    let calldata = hex::decode("55241077000000000000000000000000000000000000000000000000000000000000002a")
        .unwrap();
    let metadata_hash = [0u8; 32];
    let nonce = 1u64;
    let timestamp = 1735200000u64;
    let domain: [u8; 32] = Keccak256::digest(b"test-app").into();

    // Compute using Rust
    let rust_message_id = compute_message_id(
        message_type,
        &calldata,
        &metadata_hash,
        nonce,
        timestamp,
        &domain,
    );

    // Compute using contract
    let contract_result = contract
        .computeMessageId(
            message_type.to_string(),
            calldata.clone().into(),
            FixedBytes::from(metadata_hash),
            nonce,
            timestamp,
            FixedBytes::from(domain),
        )
        .call()
        .await
        .expect("Failed to call computeMessageId");

    let contract_message_id: [u8; 32] = contract_result.into();

    assert_eq!(
        rust_message_id, contract_message_id,
        "Message ID computation mismatch:\nRust: {}\nContract: {}",
        hex::encode(rust_message_id),
        hex::encode(contract_message_id)
    );

    println!("✓ Message ID computation matches contract");
}

#[tokio::test]
#[ignore = "requires anvil with deployed contracts"]
async fn test_e2e_domain_separator_matches_contract() {
    let bridge_address = get_bridge_address()
        .await
        .expect("BRIDGE_ADDRESS not set. Run setup-e2e-test.sh first.");

    let url: reqwest::Url = ANVIL_URL.parse().unwrap();
    let provider = ProviderBuilder::new().connect_http(url);
    let contract = IMessageBridge::new(bridge_address, &provider);

    // Get domain separator from contract
    let contract_separator = contract
        .DOMAIN_SEPARATOR()
        .call()
        .await
        .expect("Failed to get DOMAIN_SEPARATOR");

    // Compute using Rust
    let rust_separator = compute_domain_separator(ANVIL_CHAIN_ID, bridge_address);

    let contract_separator_bytes: [u8; 32] = contract_separator.into();

    assert_eq!(
        rust_separator, contract_separator_bytes,
        "Domain separator mismatch:\nRust: {}\nContract: {}",
        hex::encode(rust_separator),
        hex::encode(contract_separator_bytes)
    );

    println!("✓ Domain separator matches contract");
}

#[tokio::test]
#[ignore = "requires anvil with deployed contracts"]
async fn test_e2e_signature_verification() {
    let bridge_address = get_bridge_address()
        .await
        .expect("BRIDGE_ADDRESS not set. Run setup-e2e-test.sh first.");

    // Create signer with validator private key
    let signer = MessageSigner::new(VALIDATOR_PRIVATE_KEY, ANVIL_CHAIN_ID, bridge_address)
        .expect("Failed to create signer");

    // Create a test message
    let domain: [u8; 32] = Keccak256::digest(b"test-app").into();
    let calldata = hex::decode("55241077000000000000000000000000000000000000000000000000000000000000002a")
        .unwrap();
    let metadata_hash = Keccak256::digest(b"{}").into();
    let nonce = 1u64;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let message_id = compute_message_id(
        "setValue(uint256)",
        &calldata,
        &metadata_hash,
        nonce,
        timestamp,
        &domain,
    );

    let message = Message {
        id: message_id,
        message_type: "setValue(uint256)".to_string(),
        calldata,
        metadata: serde_json::json!({}),
        metadata_hash,
        nonce,
        timestamp,
        domain,
        value: None,
    };

    // Sign the message
    let signature = signer
        .sign_message(&message)
        .await
        .expect("Failed to sign message");

    // Verify signature structure
    assert_eq!(signature.len(), 65, "Signature should be 65 bytes (r + s + v)");

    // The v value should be 27 or 28 (or 0/1 in EIP-155)
    let v = signature[64];
    assert!(
        v == 27 || v == 28 || v == 0 || v == 1,
        "Invalid v value: {}",
        v
    );

    println!("✓ Signature generation works correctly");
    println!("  Signer address: {}", signer.address());
    println!("  Message ID: 0x{}", hex::encode(message_id));
    println!("  Signature: 0x{}", hex::encode(&signature));
}

#[tokio::test]
#[ignore = "requires anvil with deployed contracts"]
async fn test_e2e_bridge_client_queries() {
    let bridge_address = get_bridge_address()
        .await
        .expect("BRIDGE_ADDRESS not set. Run setup-e2e-test.sh first.");

    let client = BridgeClient::new(ANVIL_URL, bridge_address, VALIDATOR_PRIVATE_KEY)
        .expect("Failed to create bridge client");

    // Test domain separator query
    let domain_separator = client
        .get_domain_separator()
        .await
        .expect("Failed to get domain separator");
    assert_ne!(domain_separator, [0u8; 32], "Domain separator should not be zero");

    // Test signature threshold query
    let threshold = client
        .get_signature_threshold()
        .await
        .expect("Failed to get signature threshold");
    assert!(threshold > 0, "Threshold should be > 0");

    // Test last nonce query
    let domain: [u8; 32] = Keccak256::digest(b"test-app").into();
    let nonce = client
        .get_last_nonce(domain)
        .await
        .expect("Failed to get last nonce");
    // Nonce starts at 0
    println!("  Last nonce for domain: {}", nonce);

    println!("✓ Bridge client queries work correctly");
    println!("  Domain separator: 0x{}", hex::encode(domain_separator));
    println!("  Signature threshold: {}", threshold);
}

