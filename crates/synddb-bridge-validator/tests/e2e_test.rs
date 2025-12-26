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

#[tokio::test]
#[ignore = "requires anvil with deployed contracts"]
async fn test_e2e_full_message_submission_flow() {
    let bridge_address = get_bridge_address()
        .await
        .expect("BRIDGE_ADDRESS not set. Run setup-e2e-test.sh first.");

    // Create clients for primary validator
    let primary_client = BridgeClient::new(ANVIL_URL, bridge_address, VALIDATOR_PRIVATE_KEY)
        .expect("Failed to create primary bridge client");

    let primary_signer = MessageSigner::new(VALIDATOR_PRIVATE_KEY, ANVIL_CHAIN_ID, bridge_address)
        .expect("Failed to create primary signer");

    // Get the domain for test-app
    let domain: [u8; 32] = Keccak256::digest(b"test-app").into();

    // Get the next nonce
    let last_nonce = primary_client
        .get_last_nonce(domain)
        .await
        .expect("Failed to get last nonce");
    let nonce = last_nonce + 1;

    // Create message
    let message_type = "setValue(uint256)";
    // Encode setValue(42) - selector 0x55241077 + uint256(42)
    let calldata = hex::decode("55241077000000000000000000000000000000000000000000000000000000000000002a")
        .unwrap();
    let metadata = serde_json::json!({"test": true});
    let metadata_bytes = serde_json::to_vec(&metadata).unwrap();
    let metadata_hash: [u8; 32] = Keccak256::digest(&metadata_bytes).into();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Compute message ID
    let message_id = compute_message_id(
        message_type,
        &calldata,
        &metadata_hash,
        nonce,
        timestamp,
        &domain,
    );

    let message = Message {
        id: message_id,
        message_type: message_type.to_string(),
        calldata,
        metadata,
        metadata_hash,
        nonce,
        timestamp,
        domain,
        value: None,
    };

    println!("  Message ID: 0x{}", hex::encode(message_id));
    println!("  Nonce: {}", nonce);
    println!("  Timestamp: {}", timestamp);

    // Sign the message
    let signature = primary_signer
        .sign_message(&message)
        .await
        .expect("Failed to sign message");

    println!("  Signature: 0x{}", hex::encode(&signature));
    println!("  Signer: {}", primary_signer.address());

    // Submit message with initialize_and_sign
    primary_client
        .initialize_and_sign(&message, "memory://test", &signature, None)
        .await
        .expect("Failed to initialize and sign message");

    println!("  Message submitted to bridge");

    // Verify message stage (should be Initialized = 1 or higher)
    let stage = primary_client
        .get_message_stage(message_id)
        .await
        .expect("Failed to get message stage");
    assert!(stage >= 1, "Message should be initialized, got stage {}", stage);
    println!("  Message stage: {}", stage);

    // Verify signature count
    let sig_count = primary_client
        .get_signature_count(message_id)
        .await
        .expect("Failed to get signature count");
    assert_eq!(sig_count, 1, "Should have 1 signature from primary validator");
    println!("  Signature count: {}", sig_count);

    // Verify validator signed
    let signed = primary_client
        .has_validator_signed(message_id, primary_signer.address())
        .await
        .expect("Failed to check if validator signed");
    assert!(signed, "Primary validator should have signed");

    println!("✓ Full message submission flow works correctly");
}

#[tokio::test]
#[ignore = "requires anvil with deployed contracts"]
async fn test_e2e_witness_signing() {
    let bridge_address = get_bridge_address()
        .await
        .expect("BRIDGE_ADDRESS not set. Run setup-e2e-test.sh first.");

    // Create clients
    let primary_client = BridgeClient::new(ANVIL_URL, bridge_address, VALIDATOR_PRIVATE_KEY)
        .expect("Failed to create primary bridge client");
    let witness_client = BridgeClient::new(ANVIL_URL, bridge_address, WITNESS_PRIVATE_KEY)
        .expect("Failed to create witness bridge client");

    let primary_signer = MessageSigner::new(VALIDATOR_PRIVATE_KEY, ANVIL_CHAIN_ID, bridge_address)
        .expect("Failed to create primary signer");
    let witness_signer = MessageSigner::new(WITNESS_PRIVATE_KEY, ANVIL_CHAIN_ID, bridge_address)
        .expect("Failed to create witness signer");

    // Get the domain for test-app
    let domain: [u8; 32] = Keccak256::digest(b"test-app").into();

    // Get the next nonce
    let last_nonce = primary_client
        .get_last_nonce(domain)
        .await
        .expect("Failed to get last nonce");
    let nonce = last_nonce + 1;

    // Create message
    let message_type = "setValue(uint256)";
    let calldata = hex::decode("55241077000000000000000000000000000000000000000000000000000000000000002b")
        .unwrap(); // setValue(43)
    let metadata = serde_json::json!({"witness_test": true});
    let metadata_bytes = serde_json::to_vec(&metadata).unwrap();
    let metadata_hash: [u8; 32] = Keccak256::digest(&metadata_bytes).into();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let message_id = compute_message_id(
        message_type,
        &calldata,
        &metadata_hash,
        nonce,
        timestamp,
        &domain,
    );

    let message = Message {
        id: message_id,
        message_type: message_type.to_string(),
        calldata,
        metadata,
        metadata_hash,
        nonce,
        timestamp,
        domain,
        value: None,
    };

    println!("  Message ID: 0x{}", hex::encode(message_id));

    // Primary validator signs and initializes
    let primary_signature = primary_signer
        .sign_message(&message)
        .await
        .expect("Failed to sign message with primary");

    primary_client
        .initialize_and_sign(&message, "memory://test", &primary_signature, None)
        .await
        .expect("Failed to initialize and sign message");

    println!("  Primary validator submitted message");

    // Witness validator signs
    let witness_signature = witness_signer
        .sign_message(&message)
        .await
        .expect("Failed to sign message with witness");

    witness_client
        .sign_message(message_id, &witness_signature)
        .await
        .expect("Failed to submit witness signature");

    println!("  Witness validator signed message");

    // Verify both signatures
    let sig_count = primary_client
        .get_signature_count(message_id)
        .await
        .expect("Failed to get signature count");
    assert_eq!(sig_count, 2, "Should have 2 signatures (primary + witness)");

    let primary_signed = primary_client
        .has_validator_signed(message_id, primary_signer.address())
        .await
        .expect("Failed to check primary signature");
    assert!(primary_signed, "Primary should have signed");

    let witness_signed = primary_client
        .has_validator_signed(message_id, witness_signer.address())
        .await
        .expect("Failed to check witness signature");
    assert!(witness_signed, "Witness should have signed");

    // Check if threshold is met (threshold is 2)
    let threshold = primary_client
        .get_signature_threshold()
        .await
        .expect("Failed to get threshold");

    let stage = primary_client
        .get_message_stage(message_id)
        .await
        .expect("Failed to get message stage");

    println!("  Signature count: {} / {} threshold", sig_count, threshold);
    println!("  Message stage: {}", stage);

    // Stage should be 2 (Signed) if threshold is met
    if sig_count >= threshold {
        assert!(stage >= 2, "Message should be Signed stage when threshold met");
        println!("✓ Witness signing flow works - threshold met!");
    } else {
        println!("✓ Witness signing flow works - waiting for more signatures");
    }
}

