
use super::*;
use crate::sync::providers::mock::MockFetcher;
use alloy::signers::local::PrivateKeySigner;
use rusqlite::session::Session;
use std::io::Write;
use synddb_shared::types::{
    message::SignedMessage,
    payloads::{ChangesetBatchRequest, ChangesetData},
};

/// Test-only helper: run the sync loop until shutdown
impl Validator {
    pub async fn run(&mut self) -> Result<()> {
        self.run_with_callbacks(|_| {}, |_| {}).await
    }
}

// Test private key (DO NOT use in production!)
// Address: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
const TEST_PRIVATE_KEY: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

fn test_sequencer_pubkey() -> [u8; 64] {
    use alloy::signers::local::PrivateKeySigner;
    let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
    let pubkey = signer.credential().verifying_key().to_encoded_point(false);
    let bytes = pubkey.as_bytes();
    let mut result = [0u8; 64];
    result.copy_from_slice(&bytes[1..65]);
    result
}

// Get signer's 64-byte uncompressed public key (without 0x04 prefix)
fn signer_pubkey_bytes(signer: &PrivateKeySigner) -> [u8; 64] {
    let pubkey = signer.credential().verifying_key().to_encoded_point(false);
    let bytes = pubkey.as_bytes();
    let mut result = [0u8; 64];
    result.copy_from_slice(&bytes[1..65]);
    result
}

fn create_signed_changeset_message(sequence: u64, changeset_data: Vec<u8>) -> SignedMessage {
    use alloy::{
        primitives::{keccak256, B256},
        signers::{local::PrivateKeySigner, SignerSync},
    };
    use k256::ecdsa::Signature;
    use synddb_shared::types::cbor::{
        error::CborError,
        message::{CborMessageType, CborSignedMessage},
        verify::{signature_from_bytes, verifying_key_from_bytes},
    };

    let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
    let timestamp = 1700000000 + sequence;

    let pubkey_bytes = signer_pubkey_bytes(&signer);
    let pubkey = verifying_key_from_bytes(&pubkey_bytes).unwrap();

    // Create batch
    let batch = ChangesetBatchRequest {
        batch_id: format!("batch-{sequence}"),
        changesets: vec![ChangesetData {
            data: changeset_data,
            sequence,
            timestamp,
        }],
        attestation_token: None,
    };

    // Serialize to CBOR and compress
    let mut cbor = Vec::new();
    ciborium::into_writer(&batch, &mut cbor).unwrap();
    let mut encoder = zstd::Encoder::new(Vec::new(), 3).unwrap();
    encoder.write_all(&cbor).unwrap();
    let compressed = encoder.finish().unwrap();

    // COSE sign function
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

    // Create COSE-signed message
    let cbor_msg = CborSignedMessage::new(
        sequence,
        timestamp,
        CborMessageType::Changeset,
        compressed,
        &pubkey,
        |data| sign_cose(&signer, data),
    )
    .unwrap();

    cbor_msg.to_signed_message(&pubkey).unwrap()
}

/// Create a changeset for testing
fn create_update_changeset(conn: &Connection, new_name: &str) -> Vec<u8> {
    let mut session = Session::new(conn).unwrap();
    session.attach(None::<&str>).unwrap();

    conn.execute("UPDATE users SET name = ? WHERE id = 1", [new_name])
        .unwrap();

    let mut output = Vec::new();
    session.changeset_strm(&mut output).unwrap();
    output
}

#[tokio::test]
async fn test_validator_sync_one() {
    // Setup source database
    let source = Connection::open_in_memory().unwrap();
    source
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    source
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // Create changeset
    let changeset = create_update_changeset(&source, "Bob");

    // Create mock fetcher
    let fetcher = MockFetcher::new();

    // Create signed message
    let message = create_signed_changeset_message(0, changeset);
    fetcher.add_message(message);

    // Create validator
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut validator =
        Validator::in_memory(Arc::new(fetcher), test_sequencer_pubkey(), shutdown_rx).unwrap();

    // Setup target database with same schema
    validator
        .connection()
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    validator
        .connection()
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // Sync
    let result = validator.sync_one(0).await.unwrap();
    assert!(result);

    // Verify change was applied
    let name: String = validator
        .connection()
        .query_row("SELECT name FROM users WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "Bob");

    // Verify state was updated
    assert_eq!(validator.last_sequence().unwrap(), Some(0));
}

#[tokio::test]
async fn test_validator_sync_not_available() {
    let fetcher = MockFetcher::new();

    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut validator =
        Validator::in_memory(Arc::new(fetcher), test_sequencer_pubkey(), shutdown_rx).unwrap();

    // Try to sync non-existent message
    let result = validator.sync_one(0).await.unwrap();
    assert!(!result);
}

#[tokio::test]
async fn test_validator_sync_to_head() {
    // Setup source database
    let source = Connection::open_in_memory().unwrap();
    source
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    source
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // Create mock fetcher with multiple messages
    let fetcher = MockFetcher::new();

    // First changeset
    let changeset1 = create_update_changeset(&source, "Bob");
    let message1 = create_signed_changeset_message(0, changeset1);
    fetcher.add_message(message1);

    // Second changeset
    let changeset2 = create_update_changeset(&source, "Charlie");
    let message2 = create_signed_changeset_message(1, changeset2);
    fetcher.add_message(message2);

    // Create validator
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut validator =
        Validator::in_memory(Arc::new(fetcher), test_sequencer_pubkey(), shutdown_rx).unwrap();

    // Setup target database
    validator
        .connection()
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    validator
        .connection()
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // Sync to head
    let synced = validator.sync_to_head().await.unwrap();
    assert_eq!(synced, 2);

    // Verify final state
    let name: String = validator
        .connection()
        .query_row("SELECT name FROM users WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "Charlie");

    // Verify last sequence
    assert_eq!(validator.last_sequence().unwrap(), Some(1));
}

#[tokio::test]
async fn test_validator_run_with_shutdown() {
    let fetcher = MockFetcher::new();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut validator =
        Validator::in_memory(Arc::new(fetcher), test_sequencer_pubkey(), shutdown_rx).unwrap();

    // Spawn the run loop
    let handle = tokio::spawn(async move {
        validator.run().await.unwrap();
    });

    // Wait a bit then signal shutdown
    tokio::time::sleep(Duration::from_millis(50)).await;
    shutdown_tx.send(true).unwrap();

    // Should exit cleanly
    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("Validator should shut down within timeout")
        .unwrap();
}

// =========================================================================
// Batch sync tests
// =========================================================================

fn create_signed_batch(start_sequence: u64, messages: Vec<SignedMessage>) -> SignedBatch {
    use alloy::{
        primitives::{keccak256, B256},
        signers::{local::PrivateKeySigner, SignerSync},
    };

    let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
    let end_sequence = messages.last().map_or(start_sequence, |m| m.sequence);

    // Get 64-byte uncompressed public key
    let pubkey = signer.credential().verifying_key().to_encoded_point(false);
    let pubkey_bytes = &pubkey.as_bytes()[1..65]; // Skip 0x04 prefix

    // Create content hash from serialized messages (CBOR format)
    let content_hash = {
        let mut cbor = Vec::new();
        ciborium::into_writer(&messages, &mut cbor).expect("serialize messages");
        let hash = keccak256(&cbor);
        let mut arr = [0u8; 32];
        arr.copy_from_slice(hash.as_slice());
        arr
    };

    // Compute batch signing payload: keccak256(keccak256(start || end || content_hash))
    let mut payload_data = Vec::new();
    payload_data.extend_from_slice(&start_sequence.to_be_bytes());
    payload_data.extend_from_slice(&end_sequence.to_be_bytes());
    payload_data.extend_from_slice(&content_hash);
    let batch_payload = keccak256(&payload_data);
    let signing_payload = keccak256(batch_payload);

    // Sign with the signer's private key
    let signature = signer.sign_hash_sync(&B256::from(signing_payload)).unwrap();

    // Format as 64-byte signature (r || s) for CBOR-style
    let mut sig_bytes = [0u8; 64];
    sig_bytes[..32].copy_from_slice(&signature.r().to_be_bytes::<32>());
    sig_bytes[32..].copy_from_slice(&signature.s().to_be_bytes::<32>());

    SignedBatch {
        start_sequence,
        end_sequence,
        messages,
        batch_signature: format!("0x{}", hex::encode(sig_bytes)),
        signer: format!("0x{}", hex::encode(pubkey_bytes)),
        created_at: 1700000000,
        content_hash,
    }
}

#[tokio::test]
async fn test_validator_batch_mode_detection() {
    // Non-batch mode fetcher
    let fetcher = MockFetcher::new();
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let validator =
        Validator::in_memory(Arc::new(fetcher), test_sequencer_pubkey(), shutdown_rx).unwrap();
    assert!(!validator.supports_batch_sync());

    // Batch mode fetcher
    let fetcher = MockFetcher::new_batch_mode();
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let validator =
        Validator::in_memory(Arc::new(fetcher), test_sequencer_pubkey(), shutdown_rx).unwrap();
    assert!(validator.supports_batch_sync());
}

#[tokio::test]
async fn test_validator_sync_batch() {
    // Setup source database
    let source = Connection::open_in_memory().unwrap();
    source
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    source
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // Create multiple changesets
    let changeset0 = create_update_changeset(&source, "Bob");
    let changeset1 = create_update_changeset(&source, "Charlie");

    // Create signed messages
    let msg0 = create_signed_changeset_message(0, changeset0);
    let msg1 = create_signed_changeset_message(1, changeset1);

    // Create a batch containing both messages
    let batch = create_signed_batch(0, vec![msg0, msg1]);

    // Create mock fetcher in batch mode
    let fetcher = MockFetcher::new_batch_mode();
    fetcher.add_batch(batch.clone());

    // Create validator
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut validator =
        Validator::in_memory(Arc::new(fetcher), test_sequencer_pubkey(), shutdown_rx).unwrap();

    // Setup target database
    validator
        .connection()
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    validator
        .connection()
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // Sync the batch
    let synced = validator.sync_batch(&batch, &mut |_| {}).await.unwrap();
    assert_eq!(synced, 2);

    // Verify final state
    let name: String = validator
        .connection()
        .query_row("SELECT name FROM users WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "Charlie");

    // Verify last sequence
    assert_eq!(validator.last_sequence().unwrap(), Some(1));
}

#[tokio::test]
async fn test_validator_sync_to_head_batched() {
    // Setup source database
    let source = Connection::open_in_memory().unwrap();
    source
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    source
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // Create changesets for two batches
    let changeset0 = create_update_changeset(&source, "Bob");
    let changeset1 = create_update_changeset(&source, "Charlie");
    let changeset2 = create_update_changeset(&source, "Dave");

    // Create signed messages
    let msg0 = create_signed_changeset_message(0, changeset0);
    let msg1 = create_signed_changeset_message(1, changeset1);
    let msg2 = create_signed_changeset_message(2, changeset2);

    // Create two batches
    let batch1 = create_signed_batch(0, vec![msg0, msg1]);
    let batch2 = create_signed_batch(2, vec![msg2]);

    // Create mock fetcher in batch mode
    let fetcher = MockFetcher::new_batch_mode();
    fetcher.add_batch(batch1);
    fetcher.add_batch(batch2);

    // Create validator
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut validator =
        Validator::in_memory(Arc::new(fetcher), test_sequencer_pubkey(), shutdown_rx).unwrap();

    // Setup target database
    validator
        .connection()
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    validator
        .connection()
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // Sync to head using batched mode
    let synced = validator.sync_to_head_batched().await.unwrap();
    assert_eq!(synced, 3);

    // Verify final state
    let name: String = validator
        .connection()
        .query_row("SELECT name FROM users WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "Dave");

    // Verify last sequence
    assert_eq!(validator.last_sequence().unwrap(), Some(2));
}

#[tokio::test]
async fn test_validator_sync_to_head_batched_fallback() {
    // Setup source database
    let source = Connection::open_in_memory().unwrap();
    source
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    source
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // Create changeset
    let changeset = create_update_changeset(&source, "Bob");

    // Create mock fetcher in NON-batch mode (should fallback to single-message)
    let fetcher = MockFetcher::new();
    let message = create_signed_changeset_message(0, changeset);
    fetcher.add_message(message);

    // Create validator
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut validator =
        Validator::in_memory(Arc::new(fetcher), test_sequencer_pubkey(), shutdown_rx).unwrap();

    // Setup target database
    validator
        .connection()
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    validator
        .connection()
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // sync_to_head_batched should fallback to single-message mode
    let synced = validator.sync_to_head_batched().await.unwrap();
    assert_eq!(synced, 1);

    // Verify state
    let name: String = validator
        .connection()
        .query_row("SELECT name FROM users WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "Bob");
}

#[tokio::test]
async fn test_validator_batch_index_building() {
    let fetcher = MockFetcher::new_batch_mode();

    // Add some batches
    let batch1 = SignedBatch {
        start_sequence: 0,
        end_sequence: 49,
        messages: vec![],
        batch_signature: "0x00".to_string(),
        signer: "0x00".to_string(),
        created_at: 1700000000,
        content_hash: [0u8; 32],
    };
    let batch2 = SignedBatch {
        start_sequence: 50,
        end_sequence: 99,
        messages: vec![],
        batch_signature: "0x00".to_string(),
        signer: "0x00".to_string(),
        created_at: 1700000001,
        content_hash: [0u8; 32],
    };
    fetcher.add_batch(batch1);
    fetcher.add_batch(batch2);

    // Create validator
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let validator =
        Validator::in_memory(Arc::new(fetcher), test_sequencer_pubkey(), shutdown_rx).unwrap();

    // Build batch index
    let index = validator.build_batch_index().await.unwrap();
    assert_eq!(index.len(), 2);
    assert_eq!(index.earliest_sequence(), Some(0));
    assert_eq!(index.latest_sequence(), Some(99));

    // Test batch lookup
    assert!(index.find_batch_containing(25).is_some());
    assert!(index.find_batch_containing(75).is_some());
    assert!(index.find_batch_containing(100).is_none());
}

#[tokio::test]
async fn test_validator_batch_skip_already_synced() {
    // Setup source database
    let source = Connection::open_in_memory().unwrap();
    source
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    source
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // Create changesets
    let changeset0 = create_update_changeset(&source, "Bob");
    let changeset1 = create_update_changeset(&source, "Charlie");

    // Create signed messages
    let msg0 = create_signed_changeset_message(0, changeset0);
    let msg1 = create_signed_changeset_message(1, changeset1);

    // Create batch with both messages
    let batch = create_signed_batch(0, vec![msg0.clone(), msg1]);

    // Create mock fetcher
    let fetcher = MockFetcher::new_batch_mode();
    fetcher.add_message(msg0); // Also add as single message for initial sync
    fetcher.add_batch(batch.clone());

    // Create validator
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut validator =
        Validator::in_memory(Arc::new(fetcher), test_sequencer_pubkey(), shutdown_rx).unwrap();

    // Setup target database
    validator
        .connection()
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    validator
        .connection()
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // First, sync message 0 individually
    let synced = validator.sync_one(0).await.unwrap();
    assert!(synced);
    assert_eq!(validator.last_sequence().unwrap(), Some(0));

    // Now sync the batch - should skip message 0 and only sync message 1
    let synced = validator.sync_batch(&batch, &mut |_| {}).await.unwrap();
    assert_eq!(synced, 1); // Only message 1 should be synced

    // Verify final state
    let name: String = validator
        .connection()
        .query_row("SELECT name FROM users WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "Charlie");
}

#[tokio::test]
async fn test_validator_rejects_invalid_batch_signature() {
    // Setup source database
    let source = Connection::open_in_memory().unwrap();
    source
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    source
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // Create changeset
    let changeset = create_update_changeset(&source, "Bob");

    // Create signed message
    let msg = create_signed_changeset_message(0, changeset);

    // Create a valid batch first, then tamper with the signature
    let mut batch = create_signed_batch(0, vec![msg]);

    // Tamper with the batch signature (change last byte)
    let mut sig_bytes = hex::decode(&batch.batch_signature[2..]).unwrap();
    sig_bytes[63] ^= 0xFF; // Flip bits in the last byte
    batch.batch_signature = format!("0x{}", hex::encode(sig_bytes));

    // Create mock fetcher in batch mode
    let fetcher = MockFetcher::new_batch_mode();
    fetcher.add_batch(batch);

    // Create validator
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut validator =
        Validator::in_memory(Arc::new(fetcher), test_sequencer_pubkey(), shutdown_rx).unwrap();

    // Setup target database
    validator
        .connection()
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    validator
        .connection()
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // Sync should fail due to invalid batch signature
    let result = validator.sync_to_head_batched().await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Batch signature verification failed"),
        "Expected batch signature error, got: {err}"
    );

    // Verify no state was changed (still Alice, not Bob)
    let name: String = validator
        .connection()
        .query_row("SELECT name FROM users WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "Alice");
}

#[tokio::test]
async fn test_validator_rejects_tampered_batch_messages() {
    // Setup source database
    let source = Connection::open_in_memory().unwrap();
    source
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    source
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // Create changeset
    let changeset = create_update_changeset(&source, "Bob");

    // Create signed message
    let msg = create_signed_changeset_message(0, changeset);

    // Create a valid batch first
    let mut batch = create_signed_batch(0, vec![msg]);

    // Tamper with the message inside the batch (change the sequence)
    // This should cause the batch signature verification to fail because
    // the messages_hash will be different
    batch.messages[0].sequence = 999;

    // Create mock fetcher in batch mode
    let fetcher = MockFetcher::new_batch_mode();
    fetcher.add_batch(batch);

    // Create validator
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut validator =
        Validator::in_memory(Arc::new(fetcher), test_sequencer_pubkey(), shutdown_rx).unwrap();

    // Setup target database
    validator
        .connection()
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    validator
        .connection()
        .execute("INSERT INTO users VALUES (1, 'Alice')", [])
        .unwrap();

    // Sync should fail due to tampered message
    // With COSE format, tampering with outer sequence causes header mismatch
    let result = validator.sync_to_head_batched().await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("signature") || err.contains("Signer") || err.contains("Header mismatch"),
        "Expected signature or header mismatch error, got: {err}"
    );
}
