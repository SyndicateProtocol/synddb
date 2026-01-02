//! Core validator logic for syncing and applying state
//!
//! The `Validator` orchestrates fetching, verification, and application of
//! sequenced messages to maintain a replica of the sequenced state.

use crate::{
    apply::{
        applier::ChangesetApplier,
        audit::{DeferralReason, PendingChangeset, PendingChangesetStore},
    },
    config::ValidatorConfig,
    error::ValidatorError,
    rules::RuleRegistry,
    state::store::StateStore,
    sync::{
        batch_index::{BatchIndex, BatchIterator},
        fetcher::StorageFetcher,
        verifier::SignatureVerifier,
    },
};
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::{sync::Arc, time::Duration};
use synddb_shared::types::message::{MessageType, SignedBatch, SignedMessage};
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

/// Result of applying a message with audit trail handling
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyResult {
    /// Message was applied successfully
    Applied,
    /// Message was stored as pending due to schema mismatch (audit trail enabled)
    StoredAsPending,
}

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
    /// Pending changeset store for audit trail (always enabled)
    pending_store: PendingChangesetStore,
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
    /// Optional validation rules to apply after each changeset
    rules: Option<RuleRegistry>,
}

impl Validator {
    /// Create a new validator from configuration
    pub fn new(
        config: &ValidatorConfig,
        fetcher: Arc<dyn StorageFetcher>,
        shutdown_rx: watch::Receiver<bool>,
    ) -> Result<Self> {
        // Parse sequencer public key from hex
        let pubkey = config
            .sequencer_pubkey
            .as_ref()
            .context("sequencer_pubkey must be set (call resolve_sequencer_pubkey first)")?;
        let verifier =
            SignatureVerifier::from_hex(pubkey).context("Invalid sequencer public key")?;

        // Initialize components
        let applier = ChangesetApplier::new(&config.database_path)?;
        let state = StateStore::new(&config.state_db_path)?;

        // Initialize pending changeset store (always enabled for audit trail)
        let pending_conn = if config.pending_changesets_db_path == ":memory:" {
            Connection::open_in_memory()
        } else {
            Connection::open(&config.pending_changesets_db_path)
        }
        .context("Failed to open pending changesets database")?;
        let pending_store = PendingChangesetStore::new(pending_conn)?;

        info!(
            sequencer_pubkey = %pubkey,
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
            pending_store,
            sync_interval: config.sync_interval,
            shutdown_rx,
            gap_retry_count: config.gap_retry_count,
            gap_retry_delay: config.gap_retry_delay,
            gap_skip_on_failure: config.gap_skip_on_failure,
            batch_sync_enabled: config.batch_sync_enabled,
            batch_index_refresh_interval: config.batch_index_refresh_interval,
            rules: None,
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

        // Create in-memory pending store for testing (always enabled)
        let pending_conn = Connection::open_in_memory()
            .context("Failed to open in-memory pending changesets database")?;
        let pending_store = PendingChangesetStore::new(pending_conn)?;

        Ok(Self {
            fetcher,
            verifier,
            applier,
            state,
            pending_store,
            sync_interval: Duration::from_millis(100),
            shutdown_rx,
            gap_retry_count: 3,
            gap_retry_delay: Duration::from_millis(100),
            gap_skip_on_failure: false,
            batch_sync_enabled: true,
            batch_index_refresh_interval: Duration::from_secs(1),
            rules: None,
        })
    }

    /// Set the validation rules for this validator
    ///
    /// Rules are applied after each changeset is applied but before the transaction
    /// is committed. If any rule fails, the changeset is rejected.
    ///
    /// This is typically called by custom validator implementations to register
    /// application-specific validation logic.
    pub fn set_rules(&mut self, rules: RuleRegistry) {
        info!(rule_count = rules.len(), "Setting validation rules");
        self.rules = Some(rules);
    }

    /// Get a reference to the current rules (if any)
    pub const fn rules(&self) -> Option<&RuleRegistry> {
        self.rules.as_ref()
    }

    /// Get the last synced sequence number
    ///
    /// Returns `None` if no messages have been synced yet.
    pub fn last_sequence(&self) -> Result<Option<u64>> {
        self.state.last_sequence()
    }

    /// Get a reference to the database connection (for queries)
    pub const fn connection(&self) -> &Connection {
        &self.applier.conn
    }

    /// Get the count of pending changesets awaiting verification
    pub fn pending_changeset_count(&self) -> Result<u64> {
        self.pending_store.count()
    }

    /// Apply a message with schema mismatch handling
    ///
    /// If the message fails due to schema mismatch, the changeset is stored for
    /// later verification when a snapshot arrives, and `Ok(ApplyResult::StoredAsPending)`
    /// is returned to allow sync to continue.
    ///
    /// For snapshot messages, pending changesets are verified after application.
    ///
    /// Returns:
    /// - `Ok(ApplyResult::Applied)` - message was applied to the database
    /// - `Ok(ApplyResult::StoredAsPending)` - changeset stored for later verification
    /// - `Err(...)` - non-recoverable error
    fn apply_message_with_audit(&mut self, message: &SignedMessage) -> Result<ApplyResult> {
        let result = self
            .applier
            .apply_message_with_rules(message, self.rules.as_ref());

        match result {
            Ok(()) => {
                // If this was a snapshot, verify pending changesets
                if message.message_type == MessageType::Snapshot {
                    self.verify_pending_changesets_after_snapshot(message.sequence)?;
                }
                Ok(ApplyResult::Applied)
            }
            Err(e) => {
                let error_msg = e.to_string();

                // Check if this is a schema mismatch error
                let is_schema_mismatch = error_msg.contains("Schema mismatch")
                    || error_msg.contains("don't exist")
                    || error_msg.contains("column");

                if is_schema_mismatch && message.message_type == MessageType::Changeset {
                    // Store as pending for later verification
                    let reason = if error_msg.contains("don't exist") {
                        // Extract table name from error if possible
                        DeferralReason::MissingTable(
                            error_msg
                                .split(':')
                                .next_back()
                                .unwrap_or("unknown")
                                .trim()
                                .to_string(),
                        )
                    } else {
                        DeferralReason::ColumnMismatch {
                            table: "unknown".to_string(),
                            expected: 0,
                            actual: 0,
                        }
                    };

                    let pending = PendingChangeset {
                        sequence: message.sequence,
                        data: message.payload.clone(),
                        reason,
                    };

                    self.pending_store.store(&pending)?;

                    // Check pending count and warn if accumulating
                    let pending_count = self.pending_store.count().unwrap_or(0);
                    if pending_count > 100 {
                        error!(
                            pending_count,
                            sequence = message.sequence,
                            "Large number of pending changesets - snapshot required urgently"
                        );
                    } else if pending_count > 10 {
                        warn!(
                            pending_count,
                            sequence = message.sequence,
                            "Pending changesets accumulating - waiting for snapshot"
                        );
                    } else {
                        warn!(
                            sequence = message.sequence,
                            error = %error_msg,
                            "Stored changeset as pending due to schema mismatch - sync will continue"
                        );
                    }

                    // Return success - changeset is stored for later verification
                    return Ok(ApplyResult::StoredAsPending);
                }

                // Not a schema mismatch - propagate error
                Err(e)
            }
        }
    }

    /// Verify pending changesets after a snapshot has been applied
    fn verify_pending_changesets_after_snapshot(&self, snapshot_sequence: u64) -> Result<()> {
        let pending_count = self.pending_store.count()?;
        if pending_count == 0 {
            return Ok(());
        }

        info!(
            snapshot_sequence,
            pending_count, "Verifying pending changesets after snapshot"
        );

        // Get pending changesets up to this snapshot
        let pending = self.pending_store.get_all()?;
        let relevant: Vec<_> = pending
            .into_iter()
            .filter(|p| p.sequence < snapshot_sequence)
            .collect();

        if relevant.is_empty() {
            return Ok(());
        }

        // Verify the changesets
        let result = crate::apply::audit::verify_changeset_chain(&self.applier.conn, &relevant)?;

        info!(
            verified = result.verified.len(),
            failed = result.failed.len(),
            "Pending changeset verification complete"
        );

        for failure in &result.failed {
            warn!(
                sequence = failure.sequence,
                reason = %failure.reason,
                "Changeset verification failed"
            );
        }

        // Clear verified changesets up to the snapshot
        self.pending_store.clear_up_to(snapshot_sequence)?;

        Ok(())
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
                                crate::metrics::record_gap_detected();

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
        let sync_start = std::time::Instant::now();

        // 1. Fetch message
        let fetch_start = std::time::Instant::now();
        let message = match self.fetcher.get(sequence).await? {
            Some(msg) => msg,
            None => return Ok(false),
        };
        crate::metrics::record_fetch_duration(fetch_start.elapsed().as_secs_f64());

        debug!(
            sequence,
            message_type = ?message.message_type,
            "Fetched message"
        );

        // 2. Validate sequence matches what we requested (defensive check)
        if message.sequence != sequence {
            return Err(ValidatorError::SequenceGap {
                expected: sequence,
                actual: message.sequence,
            }
            .into());
        }

        // 3. Verify signature
        let verify_start = std::time::Instant::now();
        self.verifier
            .verify(&message)
            .map_err(|e| ValidatorError::SignatureVerification(e.to_string()))?;
        crate::metrics::record_verify_duration(verify_start.elapsed().as_secs_f64());

        debug!(sequence, "Signature verified");

        // 4. Check for withdrawal and call callback
        if let Some(withdrawal) = ChangesetApplier::extract_withdrawal(&message)? {
            debug!(
                sequence,
                request_id = %withdrawal.request_id,
                recipient = %withdrawal.recipient,
                amount = %withdrawal.amount,
                "Processing withdrawal message"
            );
            on_withdrawal(&withdrawal);
            crate::metrics::record_withdrawal_processed();
        }

        // 5. Apply to database with audit trail handling
        let apply_start = std::time::Instant::now();
        let apply_result = self.apply_message_with_audit(&message)?;
        let message_type_str = match message.message_type {
            MessageType::Changeset => "changeset",
            MessageType::Withdrawal => "withdrawal",
            MessageType::Snapshot => "snapshot",
        };
        crate::metrics::record_apply_duration(
            message_type_str,
            apply_start.elapsed().as_secs_f64(),
        );

        match apply_result {
            ApplyResult::Applied => {
                debug!(sequence, "Message applied");
            }
            ApplyResult::StoredAsPending => {
                debug!(sequence, "Message stored as pending (schema mismatch)");
                if let Ok(count) = self.pending_changeset_count() {
                    crate::metrics::update_pending_changesets(count);
                }
            }
        }

        // 6. Update state (even if stored as pending - we've processed this sequence)
        self.state.record_sync(message.sequence)?;

        // Record metrics
        crate::metrics::record_message_synced(message_type_str, message.sequence);
        crate::metrics::record_sync_duration(sync_start.elapsed().as_secs_f64());

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
                    // Errors (including sequence mismatches and unavailable data) stop sync.
                    // This is correct behavior - re-derivation should halt on any error.
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

            // Apply to database with audit trail handling
            let apply_result = self.apply_message_with_audit(message)?;

            match apply_result {
                ApplyResult::Applied => {
                    debug!(sequence = message.sequence, "Message applied");
                }
                ApplyResult::StoredAsPending => {
                    debug!(
                        sequence = message.sequence,
                        "Message stored as pending (schema mismatch)"
                    );
                }
            }

            // Update state (even if stored as pending)
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

                            // Apply with audit trail handling
                            match self.apply_message_with_audit(message) {
                                Ok(ApplyResult::Applied) => {
                                    debug!(sequence = message.sequence, "Message applied");
                                }
                                Ok(ApplyResult::StoredAsPending) => {
                                    debug!(
                                        sequence = message.sequence,
                                        "Message stored as pending (schema mismatch)"
                                    );
                                }
                                Err(e) => {
                                    error!(
                                        sequence = message.sequence,
                                        error = %e,
                                        "Failed to apply message"
                                    );
                                    return Err(e);
                                }
                            }

                            // Record sync (even if stored as pending)
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
#[path = "validator_tests.rs"]
mod tests;
