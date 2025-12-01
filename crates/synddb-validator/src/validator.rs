//! Core validator logic for syncing and applying state
//!
//! The `Validator` orchestrates fetching, verification, and application of
//! sequenced messages to maintain a replica of the sequenced state.

use crate::apply::ChangesetApplier;
use crate::config::ValidatorConfig;
use crate::error::ValidatorError;
use crate::state::StateStore;
use crate::sync::{DAFetcher, SignatureVerifier};
use alloy::primitives::Address;
use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

/// Core validator that syncs and applies state from the sequencer
pub struct Validator {
    /// DA fetcher for retrieving messages
    fetcher: Arc<dyn DAFetcher>,
    /// Signature verifier for message authentication
    verifier: SignatureVerifier,
    /// Changeset applier for state reconstruction
    applier: ChangesetApplier,
    /// State persistence for crash recovery
    state: StateStore,
    /// Sync poll interval
    sync_interval: Duration,
    /// Shutdown receiver
    shutdown_rx: watch::Receiver<bool>,
    /// Gap retry count
    gap_retry_count: u32,
    /// Gap retry delay
    gap_retry_delay: Duration,
    /// Skip gaps after max retries
    gap_skip_on_failure: bool,
}

impl Validator {
    /// Create a new validator from configuration
    pub fn new(
        config: &ValidatorConfig,
        fetcher: Arc<dyn DAFetcher>,
        shutdown_rx: watch::Receiver<bool>,
    ) -> Result<Self> {
        // Parse sequencer address
        let sequencer_address: Address = config
            .sequencer_address
            .parse()
            .context("Invalid sequencer address")?;

        // Initialize components
        let verifier = SignatureVerifier::new(sequencer_address);
        let applier = ChangesetApplier::new(&config.database_path)?;
        let state = StateStore::new(&config.state_db_path)?;

        info!(
            sequencer = %config.sequencer_address,
            database = %config.database_path,
            gap_retry_count = config.gap_retry_count,
            gap_skip_on_failure = config.gap_skip_on_failure,
            "Validator initialized"
        );

        Ok(Self {
            fetcher,
            verifier,
            applier,
            state,
            sync_interval: config.sync_interval,
            shutdown_rx,
            gap_retry_count: config.gap_retry_count,
            gap_retry_delay: config.gap_retry_delay,
            gap_skip_on_failure: config.gap_skip_on_failure,
        })
    }

    /// Create a validator for testing with in-memory storage
    pub fn in_memory(
        fetcher: Arc<dyn DAFetcher>,
        sequencer_address: Address,
        shutdown_rx: watch::Receiver<bool>,
    ) -> Result<Self> {
        let verifier = SignatureVerifier::new(sequencer_address);
        let applier = ChangesetApplier::in_memory()?;
        let state = StateStore::in_memory()?;

        Ok(Self {
            fetcher,
            verifier,
            applier,
            state,
            sync_interval: Duration::from_millis(100),
            shutdown_rx,
            gap_retry_count: 3,
            gap_retry_delay: Duration::from_millis(100),
            gap_skip_on_failure: false,
        })
    }

    /// Get the last synced sequence number
    ///
    /// Returns `None` if no messages have been synced yet.
    pub fn last_sequence(&self) -> Result<Option<u64>> {
        self.state.last_sequence()
    }

    /// Get a reference to the database connection (for queries)
    pub const fn connection(&self) -> &rusqlite::Connection {
        self.applier.connection()
    }

    /// Run the sync loop until shutdown
    pub async fn run(&mut self) -> Result<()> {
        self.run_with_callbacks(|_| {}, |_| {}).await
    }

    /// Run the sync loop with callbacks for withdrawals and progress updates
    ///
    /// - `on_withdrawal`: Called when a withdrawal message is processed
    /// - `on_sync`: Called after each successful sync with the sequence number
    pub async fn run_with_callbacks<W, S>(
        &mut self,
        mut on_withdrawal: W,
        mut on_sync: S,
    ) -> Result<()>
    where
        W: FnMut(&synddb_shared::types::WithdrawalRequest),
        S: FnMut(u64),
    {
        info!("Starting validator sync loop");

        // Get starting sequence
        let mut next_sequence = self.state.next_sequence()?;
        info!(next_sequence, "Resuming from sequence");

        // Track consecutive "not found" count for gap detection
        let mut not_found_count: u32 = 0;

        // Extract config values to avoid holding &self across await
        let gap_retry_count = self.gap_retry_count;
        let gap_retry_delay = self.gap_retry_delay;
        let gap_skip_on_failure = self.gap_skip_on_failure;
        let sync_interval = self.sync_interval;

        loop {
            // Check for shutdown
            if *self.shutdown_rx.borrow() {
                info!("Shutdown signal received, stopping sync loop");
                break;
            }

            // Try to sync next message
            match self
                .sync_one_with_callback(next_sequence, &mut on_withdrawal)
                .await
            {
                Ok(true) => {
                    // Successfully synced, reset gap counter
                    not_found_count = 0;
                    // Call progress callback
                    on_sync(next_sequence);
                    next_sequence += 1;
                    // Don't sleep - immediately try next message
                    continue;
                }
                Ok(false) => {
                    // No message available yet
                    not_found_count += 1;

                    // Check if this might be a gap (we're missing messages)
                    if not_found_count >= gap_retry_count {
                        // Try to detect if there are future messages available
                        // Clone fetcher to avoid holding &self across await
                        let fetcher = Arc::clone(&self.fetcher);
                        match Self::detect_gap_static(&fetcher, next_sequence).await {
                            Ok(Some(available_seq)) => {
                                let gap_size = available_seq - next_sequence;
                                warn!(
                                    expected = next_sequence,
                                    available = available_seq,
                                    gap_size,
                                    "Sequence gap detected"
                                );

                                if !gap_skip_on_failure {
                                    // Return error - operator needs to intervene
                                    return Err(ValidatorError::SequenceGap {
                                        expected: next_sequence,
                                        actual: available_seq,
                                    }
                                    .into());
                                }

                                warn!(
                                    skipping_from = next_sequence,
                                    skipping_to = available_seq,
                                    "Skipping gap (gap_skip_on_failure enabled)"
                                );
                                // Record the gap in state (for auditing)
                                if let Err(e) = self.state.record_gap(next_sequence, available_seq)
                                {
                                    error!(error = %e, "Failed to record gap");
                                }
                                next_sequence = available_seq;
                                not_found_count = 0;
                                continue;
                            }
                            Ok(None) => {
                                // No future messages either, just waiting for new data
                                debug!(sequence = next_sequence, "No message available, waiting");
                                not_found_count = 0; // Reset since we confirmed no gap
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to detect gap");
                            }
                        }
                    } else {
                        debug!(
                            sequence = next_sequence,
                            retry = not_found_count,
                            max_retries = gap_retry_count,
                            "No message available, will retry"
                        );
                    }
                }
                Err(e) => {
                    // Error during sync - log and continue
                    error!(sequence = next_sequence, error = %e, "Sync error");
                }
            }

            // Wait before next poll (use gap_retry_delay if we're in gap detection mode)
            let wait_duration = if not_found_count > 0 {
                gap_retry_delay
            } else {
                sync_interval
            };

            tokio::select! {
                _ = tokio::time::sleep(wait_duration) => {}
                _ = self.shutdown_rx.changed() => {
                    if *self.shutdown_rx.borrow() {
                        info!("Shutdown signal received during wait");
                        break;
                    }
                }
            }
        }

        info!("Sync loop stopped");
        Ok(())
    }

    /// Detect if there's a gap by checking for future messages (static version)
    ///
    /// Returns `Some(sequence)` if a message exists at a higher sequence number,
    /// indicating a gap. Returns `None` if no future messages are found.
    async fn detect_gap_static(
        fetcher: &Arc<dyn DAFetcher>,
        expected_sequence: u64,
    ) -> Result<Option<u64>> {
        // Check a few future sequences to see if data exists
        for offset in 1..=10 {
            let check_seq = expected_sequence + offset;
            match fetcher.get(check_seq).await {
                Ok(Some(_)) => {
                    // Found a message at a higher sequence - there's a gap
                    return Ok(Some(check_seq));
                }
                Ok(None) => {
                    // No message at this sequence either
                }
                Err(e) => {
                    debug!(sequence = check_seq, error = %e, "Error checking for gap");
                }
            }
        }

        // Also try to get the latest sequence from the fetcher
        if let Ok(Some(latest)) = fetcher.get_latest_sequence().await {
            if latest > expected_sequence {
                return Ok(Some(expected_sequence + 1)); // Return next expected
            }
        }

        Ok(None)
    }

    /// Sync a single message by sequence number
    ///
    /// Returns `Ok(true)` if message was synced, `Ok(false)` if not available yet
    pub async fn sync_one(&mut self, sequence: u64) -> Result<bool> {
        self.sync_one_with_callback(sequence, &mut |_| {}).await
    }

    /// Sync a single message by sequence number with a callback for withdrawal messages
    ///
    /// The callback receives the `WithdrawalRequest` if the message is a withdrawal.
    /// Returns `Ok(true)` if message was synced, `Ok(false)` if not available yet.
    pub async fn sync_one_with_callback<F>(
        &mut self,
        sequence: u64,
        on_withdrawal: &mut F,
    ) -> Result<bool>
    where
        F: FnMut(&synddb_shared::types::WithdrawalRequest),
    {
        // 1. Fetch message
        let message = match self.fetcher.get(sequence).await? {
            Some(msg) => msg,
            None => return Ok(false),
        };

        debug!(
            sequence,
            message_type = ?message.message_type,
            "Fetched message"
        );

        // 2. Verify signature
        self.verifier
            .verify(&message)
            .map_err(|e| ValidatorError::SignatureVerification(e.to_string()))?;

        debug!(sequence, "Signature verified");

        // 3. Check for withdrawal and call callback
        if let Some(withdrawal) = ChangesetApplier::extract_withdrawal(&message)? {
            debug!(
                sequence,
                request_id = %withdrawal.request_id,
                recipient = %withdrawal.recipient,
                amount = %withdrawal.amount,
                "Processing withdrawal message"
            );
            on_withdrawal(&withdrawal);
        }

        // 4. Apply to database (logs withdrawal, applies changeset)
        self.applier.apply_message(&message)?;

        debug!(sequence, "Message applied");

        // 5. Update state
        self.state.record_sync(sequence)?;

        info!(sequence, "Synced message");

        Ok(true)
    }

    /// Sync all available messages up to the latest
    pub async fn sync_to_head(&mut self) -> Result<u64> {
        let mut next_sequence = self.state.next_sequence()?;
        let mut synced = 0;

        loop {
            match self.sync_one(next_sequence).await {
                Ok(true) => {
                    synced += 1;
                    next_sequence += 1;
                }
                Ok(false) => {
                    // Caught up to head
                    break;
                }
                Err(e) => {
                    warn!(sequence = next_sequence, error = %e, "Sync error, stopping");
                    break;
                }
            }
        }

        if synced > 0 {
            info!(synced, last_sequence = next_sequence - 1, "Synced to head");
        }

        Ok(synced)
    }
}

impl std::fmt::Debug for Validator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Validator")
            .field("fetcher", &self.fetcher.name())
            .field("sync_interval", &self.sync_interval)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::providers::MockFetcher;
    use rusqlite::session::Session;
    use std::io::Write;
    use synddb_shared::types::{ChangesetBatchRequest, ChangesetData, MessageType, SignedMessage};

    // Test private key (DO NOT use in production!)
    // Address: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
    const TEST_PRIVATE_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    fn test_sequencer_address() -> Address {
        use alloy::signers::local::PrivateKeySigner;
        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        signer.address()
    }

    async fn create_signed_changeset_message(
        sequence: u64,
        changeset_data: Vec<u8>,
    ) -> SignedMessage {
        use alloy::primitives::keccak256;
        use alloy::signers::local::PrivateKeySigner;
        use alloy::signers::Signer;

        let signer: PrivateKeySigner = TEST_PRIVATE_KEY.parse().unwrap();
        let timestamp = 1700000000 + sequence;

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

        // Compress
        let json = serde_json::to_vec(&batch).unwrap();
        let mut encoder = zstd::Encoder::new(Vec::new(), 3).unwrap();
        encoder.write_all(&json).unwrap();
        let compressed = encoder.finish().unwrap();

        // Hash
        let message_hash = keccak256(&compressed);

        // Create signing payload
        let mut signing_data = Vec::with_capacity(48);
        signing_data.extend_from_slice(&sequence.to_be_bytes());
        signing_data.extend_from_slice(&timestamp.to_be_bytes());
        signing_data.extend_from_slice(message_hash.as_slice());
        let signing_payload = keccak256(&signing_data);

        // Sign
        let signature = signer.sign_hash(&signing_payload).await.unwrap();

        // Format signature
        let mut sig_bytes = [0u8; 65];
        sig_bytes[..32].copy_from_slice(&signature.r().to_be_bytes::<32>());
        sig_bytes[32..64].copy_from_slice(&signature.s().to_be_bytes::<32>());
        sig_bytes[64] = if signature.v() { 28 } else { 27 };

        SignedMessage {
            sequence,
            timestamp,
            message_type: MessageType::Changeset,
            payload: compressed,
            message_hash: format!("0x{}", hex::encode(message_hash)),
            signature: format!("0x{}", hex::encode(sig_bytes)),
            signer: format!("{:?}", signer.address()),
        }
    }

    /// Create a changeset for testing
    fn create_update_changeset(conn: &rusqlite::Connection, new_name: &str) -> Vec<u8> {
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
        let source = rusqlite::Connection::open_in_memory().unwrap();
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
        let message = create_signed_changeset_message(0, changeset).await;
        fetcher.add_message(message);

        // Create validator
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let mut validator =
            Validator::in_memory(Arc::new(fetcher), test_sequencer_address(), shutdown_rx).unwrap();

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
            Validator::in_memory(Arc::new(fetcher), test_sequencer_address(), shutdown_rx).unwrap();

        // Try to sync non-existent message
        let result = validator.sync_one(0).await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_validator_sync_to_head() {
        // Setup source database
        let source = rusqlite::Connection::open_in_memory().unwrap();
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
        let message1 = create_signed_changeset_message(0, changeset1).await;
        fetcher.add_message(message1);

        // Second changeset
        let changeset2 = create_update_changeset(&source, "Charlie");
        let message2 = create_signed_changeset_message(1, changeset2).await;
        fetcher.add_message(message2);

        // Create validator
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let mut validator =
            Validator::in_memory(Arc::new(fetcher), test_sequencer_address(), shutdown_rx).unwrap();

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
            Validator::in_memory(Arc::new(fetcher), test_sequencer_address(), shutdown_rx).unwrap();

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
}
