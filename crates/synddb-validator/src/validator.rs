//! Core validator logic for syncing and applying state
//!
//! The `Validator` orchestrates fetching, verification, and application of
//! sequenced messages to maintain a replica of the sequenced state.

use crate::{
    apply::applier::ChangesetApplier,
    config::ValidatorConfig,
    error::ValidatorError,
    state::store::StateStore,
    sync::{
        batch_index::{BatchIndex, BatchIterator},
        fetcher::StorageFetcher,
        verifier::SignatureVerifier,
    },
};
use anyhow::{Context, Result};
use std::{sync::Arc, time::Duration};
use synddb_shared::types::message::SignedBatch;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

/// Core validator that syncs and applies state from the sequencer
pub struct Validator {
    /// Storage fetcher for retrieving messages
    fetcher: Arc<dyn StorageFetcher>,
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
    /// Whether batch sync is enabled
    batch_sync_enabled: bool,
    /// How often to refresh the batch index
    batch_index_refresh_interval: Duration,
}

impl Validator {
    /// Create a new validator from configuration
    pub fn new(
        config: &ValidatorConfig,
        fetcher: Arc<dyn StorageFetcher>,
        shutdown_rx: watch::Receiver<bool>,
    ) -> Result<Self> {
        // Parse sequencer public key from hex
        let verifier = SignatureVerifier::from_hex(&config.sequencer_pubkey)
            .context("Invalid sequencer public key")?;

        // Initialize components
        let applier = ChangesetApplier::new(&config.database_path)?;
        let state = StateStore::new(&config.state_db_path)?;

        info!(
            sequencer_pubkey = %config.sequencer_pubkey,
            database = %config.database_path,
            gap_retry_count = config.gap_retry_count,
            gap_skip_on_failure = config.gap_skip_on_failure,
            batch_sync_enabled = config.batch_sync_enabled,
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
            batch_sync_enabled: config.batch_sync_enabled,
            batch_index_refresh_interval: config.batch_index_refresh_interval,
        })
    }

    /// Create a validator for testing with in-memory storage
    pub fn in_memory(
        fetcher: Arc<dyn StorageFetcher>,
        sequencer_pubkey: [u8; 64],
        shutdown_rx: watch::Receiver<bool>,
    ) -> Result<Self> {
        let verifier = SignatureVerifier::new(sequencer_pubkey);
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
            batch_sync_enabled: true,
            batch_index_refresh_interval: Duration::from_secs(1),
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
        &self.applier.conn
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
        W: FnMut(&synddb_shared::types::payloads::WithdrawalRequest),
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
                        match Self::detect_gap(&fetcher, next_sequence).await {
                            Ok(Some(available_seq)) => {
                                let gap_size = available_seq - next_sequence;
                                warn!(
                                    expected = next_sequence,
                                    available = available_seq,
                                    gap_size,
                                    "Sequence gap detected"
                                );

                                if !gap_skip_on_failure {
                                    error!(
                                        expected = next_sequence,
                                        available = available_seq,
                                        gap_size,
                                        "Sequence gap detected - operator intervention required"
                                    );
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

    /// Detect if there's a gap by checking for future messages
    ///
    /// Returns `Some(sequence)` if a message exists at a higher sequence number,
    /// indicating a gap. Returns `None` if no future messages are found.
    async fn detect_gap(
        fetcher: &Arc<dyn StorageFetcher>,
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
        F: FnMut(&synddb_shared::types::payloads::WithdrawalRequest),
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
                //TODO CLAUDE: make sure that "out of sequence" errors are caught here, because data may be unavailable.
                // Re-derivation should quit
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

    // =========================================================================
    // Batch sync methods (for fetchers that support batch operations)
    // =========================================================================

    /// Check if the fetcher supports batch operations
    pub fn supports_batch_sync(&self) -> bool {
        self.fetcher.supports_batches()
    }

    /// Build or refresh a batch index from the fetcher
    pub async fn build_batch_index(&self) -> Result<BatchIndex> {
        BatchIndex::build(&self.fetcher).await
    }

    /// Sync all messages from a batch, verifying and applying each
    ///
    /// Returns the number of messages synced from this batch.
    ///
    /// Signature verification is always performed using COSE format.
    pub async fn sync_batch<F>(&mut self, batch: &SignedBatch, on_withdrawal: &mut F) -> Result<u64>
    where
        F: FnMut(&synddb_shared::types::payloads::WithdrawalRequest),
    {
        // Verify batch signature before processing any messages
        batch.verify_batch_signature().map_err(|e| {
            ValidatorError::SignatureVerification(format!(
                "Batch signature verification failed: {e}"
            ))
        })?;

        let next_sequence = self.state.next_sequence()?;
        let mut synced = 0;

        for message in &batch.messages {
            // Skip messages we've already processed
            if message.sequence < next_sequence {
                debug!(
                    sequence = message.sequence,
                    next_sequence, "Skipping already-synced message"
                );
                continue;
            }

            // Verify COSE signature
            self.verifier
                .verify(message)
                .map_err(|e| ValidatorError::SignatureVerification(e.to_string()))?;

            debug!(sequence = message.sequence, "Signature verified");

            // Check for withdrawal and call callback
            if let Some(withdrawal) = ChangesetApplier::extract_withdrawal(message)? {
                debug!(
                    sequence = message.sequence,
                    request_id = %withdrawal.request_id,
                    "Processing withdrawal message"
                );
                on_withdrawal(&withdrawal);
            }

            // Apply to database
            self.applier.apply_message(message)?;

            debug!(sequence = message.sequence, "Message applied");

            // Update state
            self.state.record_sync(message.sequence)?;
            synced += 1;
        }

        if synced > 0 {
            info!(
                batch_start = batch.start_sequence,
                batch_end = batch.end_sequence,
                synced,
                "Synced batch"
            );
        }

        Ok(synced)
    }

    /// Sync to head using batch fetching (more efficient for sequential sync)
    ///
    /// Uses batch index to efficiently fetch and process messages in batches.
    /// Falls back to single-message fetching if batch sync is not supported.
    pub async fn sync_to_head_batched(&mut self) -> Result<u64> {
        if !self.fetcher.supports_batches() {
            debug!("Fetcher doesn't support batches, falling back to single-message sync");
            return self.sync_to_head().await;
        }

        let index = self.build_batch_index().await?;
        let next_sequence = self.state.next_sequence()?;
        let mut total_synced = 0;

        // Create iterator starting at our next sequence
        let mut iter = BatchIterator::starting_at(&index, next_sequence);

        while !iter.is_exhausted(&index) {
            let batch_info = match iter.current_batch(&index) {
                Some(info) => info.clone(),
                None => break,
            };

            // Fetch the batch
            match self.fetcher.get_batch_by_path(&batch_info.path).await? {
                Some(batch) => {
                    let synced = self.sync_batch(&batch, &mut |_| {}).await?;
                    total_synced += synced;
                }
                None => {
                    warn!(
                        path = batch_info.path,
                        start = batch_info.start_sequence,
                        "Batch not found"
                    );
                }
            }

            // Move to next batch
            iter.advance_to_next_batch();
        }

        if total_synced > 0 {
            info!(
                total_synced,
                batches_processed = iter.batch_index(),
                "Batched sync complete"
            );
        }

        Ok(total_synced)
    }

    /// Run the sync loop using batch mode when available
    ///
    /// This is the recommended sync loop for production use. It:
    /// - Uses batch fetching when the fetcher supports it
    /// - Falls back to single-message fetching otherwise
    /// - Handles gap detection and recovery
    pub async fn run_batched<W, S>(&mut self, mut on_withdrawal: W, mut on_sync: S) -> Result<()>
    where
        W: FnMut(&synddb_shared::types::payloads::WithdrawalRequest),
        S: FnMut(u64),
    {
        let use_batch_mode = self.batch_sync_enabled && self.fetcher.supports_batches();
        info!(
            batch_mode = use_batch_mode,
            batch_sync_enabled = self.batch_sync_enabled,
            fetcher_supports_batches = self.fetcher.supports_batches(),
            "Starting validator sync loop"
        );

        // If batch mode isn't supported or disabled, use the original loop
        if !use_batch_mode {
            return self.run_with_callbacks(on_withdrawal, on_sync).await;
        }

        // Clone fetcher Arc to avoid holding &self across await points
        let fetcher = Arc::clone(&self.fetcher);

        // Build initial batch index
        let mut index = BatchIndex::build(&fetcher).await?;
        let mut last_index_refresh = std::time::Instant::now();
        let index_refresh_interval = self.batch_index_refresh_interval;

        // Get starting sequence
        let mut next_sequence = self.state.next_sequence()?;
        info!(next_sequence, "Resuming from sequence");

        // Track if we're caught up (no new batches to process)
        let mut caught_up = false;

        // Extract config values
        let sync_interval = self.sync_interval;
        let gap_retry_delay = self.gap_retry_delay;

        loop {
            // Check for shutdown
            if *self.shutdown_rx.borrow() {
                info!("Shutdown signal received, stopping sync loop");
                break;
            }

            // Refresh index periodically
            if last_index_refresh.elapsed() >= index_refresh_interval || caught_up {
                if let Ok(new_batches) = index.refresh(&fetcher).await {
                    if new_batches > 0 {
                        debug!(new_batches, "Discovered new batches");
                        caught_up = false;
                    }
                }
                last_index_refresh = std::time::Instant::now();
            }

            // Find batch containing our next sequence
            if let Some(batch_info) = index.find_batch_containing(next_sequence) {
                let batch_info = batch_info.clone();

                // Fetch and process the batch
                match fetcher.get_batch_by_path(&batch_info.path).await {
                    Ok(Some(batch)) => {
                        // Verify batch signature before processing messages
                        if let Err(e) = batch.verify_batch_signature() {
                            error!(
                                start_sequence = batch.start_sequence,
                                end_sequence = batch.end_sequence,
                                error = %e,
                                "Batch signature verification failed"
                            );
                            return Err(ValidatorError::SignatureVerification(format!(
                                "Batch signature verification failed: {e}"
                            ))
                            .into());
                        }

                        for message in &batch.messages {
                            if message.sequence < next_sequence {
                                continue;
                            }

                            // Verify COSE signature
                            if let Err(e) = self.verifier.verify(message) {
                                error!(
                                    sequence = message.sequence,
                                    error = %e,
                                    "Signature verification failed"
                                );
                                return Err(
                                    ValidatorError::SignatureVerification(e.to_string()).into()
                                );
                            }

                            // Handle withdrawal
                            if let Ok(Some(withdrawal)) =
                                ChangesetApplier::extract_withdrawal(message)
                            {
                                on_withdrawal(&withdrawal);
                            }

                            // Apply
                            if let Err(e) = self.applier.apply_message(message) {
                                error!(
                                    sequence = message.sequence,
                                    error = %e,
                                    "Failed to apply message"
                                );
                                return Err(e);
                            }

                            // Record sync
                            self.state.record_sync(message.sequence)?;
                            on_sync(message.sequence);
                            next_sequence = message.sequence + 1;
                        }

                        // Immediately try next batch
                        continue;
                    }
                    Ok(None) => {
                        warn!(path = batch_info.path, "Batch not found in DA");
                    }
                    Err(e) => {
                        error!(error = %e, "Error fetching batch");
                    }
                }
            } else if let Some(next_batch) =
                index.find_first_batch_starting_at_or_after(next_sequence)
            {
                // Gap detected: no batch contains our sequence but there are future batches
                let gap_start = next_sequence;
                let gap_end = next_batch.start_sequence;

                warn!(
                    gap_start,
                    gap_end,
                    gap_size = gap_end - gap_start,
                    "Sequence gap detected in batch index"
                );

                if !self.gap_skip_on_failure {
                    return Err(ValidatorError::SequenceGap {
                        expected: gap_start,
                        actual: gap_end,
                    }
                    .into());
                }

                // Skip the gap
                self.state.record_gap(gap_start, gap_end)?;
                next_sequence = gap_end;
                continue;
            } else {
                // No more batches - we're caught up
                caught_up = true;
            }

            // Wait before polling again
            let wait_duration = if caught_up {
                sync_interval
            } else {
                gap_retry_delay
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
    const TEST_PRIVATE_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

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
        let source = rusqlite::Connection::open_in_memory().unwrap();
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
        let source = rusqlite::Connection::open_in_memory().unwrap();
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
        let source = rusqlite::Connection::open_in_memory().unwrap();
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
        let source = rusqlite::Connection::open_in_memory().unwrap();
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
        let source = rusqlite::Connection::open_in_memory().unwrap();
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
        let source = rusqlite::Connection::open_in_memory().unwrap();
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
}
