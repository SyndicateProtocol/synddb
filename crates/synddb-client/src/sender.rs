//! Background sender that batches and sends changesets to sequencer

use crate::attestation::AttestationClient;
use crate::config::Config;
use crate::recovery::FailedBatchRecovery;
use crate::retry::retry_with_backoff;
use crate::session::Changeset;
use crossbeam_channel::{select, Receiver};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, warn};

#[derive(Debug, Serialize, Deserialize)]
struct ChangesetBatch {
    changesets: Vec<Changeset>,
    batch_id: String,
    /// Optional TEE attestation token (JWT) proving workload identity
    #[serde(skip_serializing_if = "Option::is_none")]
    attestation_token: Option<String>,
}

#[derive(Debug)]
pub struct ChangesetSender {
    config: Config,
    client: Client,
    buffer: Vec<Changeset>,
    buffer_size: usize,
    last_flush: Instant,
    recovery: Option<Arc<FailedBatchRecovery>>,
    attestation: Option<AttestationClient>,
}

impl ChangesetSender {
    pub(crate) fn new(
        config: Config,
        recovery: Option<Arc<FailedBatchRecovery>>,
        attestation: Option<AttestationClient>,
    ) -> Self {
        let client = Client::builder()
            .timeout(config.request_timeout)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            config,
            client,
            buffer: Vec::new(),
            buffer_size: 0,
            last_flush: Instant::now(),
            recovery,
            attestation,
        }
    }

    pub(crate) async fn run(
        mut self,
        changeset_rx: Receiver<Changeset>,
        shutdown_rx: Receiver<()>,
    ) {
        info!("ChangesetSender started");

        // Retry any failed changesets from previous runs
        self.retry_failed_changesets().await;

        loop {
            select! {
                recv(changeset_rx) -> changeset => {
                    match changeset {
                        Ok(cs) => {
                            debug!("Received changeset: seq={}, size={} bytes", cs.sequence, cs.data.len());
                            self.buffer_size += cs.data.len();
                            self.buffer.push(cs);

                            // Flush if buffer is full
                            if self.should_flush() {
                                self.flush().await;
                            }
                        }
                        Err(_) => {
                            warn!("Changeset channel closed");
                            break;
                        }
                    }
                }
                recv(shutdown_rx) -> _ => {
                    info!("Shutdown signal received");
                    self.flush().await;
                    break;
                }
            }
        }

        info!("ChangesetSender stopped");
    }

    fn should_flush(&self) -> bool {
        self.buffer.len() >= self.config.buffer_size
            || self.buffer_size >= self.config.max_batch_size
            || self.last_flush.elapsed() >= self.config.publish_interval
    }

    async fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        // Obtain attestation token if configured
        let attestation_token = if let Some(ref attestation) = self.attestation {
            match attestation.get_token().await {
                Ok(token) => {
                    debug!("Obtained attestation token for changeset batch");
                    Some(token)
                }
                Err(e) => {
                    error!("Failed to obtain attestation token: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let batch = ChangesetBatch {
            changesets: std::mem::take(&mut self.buffer),
            batch_id: uuid::Uuid::new_v4().to_string(),
            attestation_token,
        };

        debug!(
            "Flushing {} changesets to sequencer (attestation: {})",
            batch.changesets.len(),
            batch.attestation_token.is_some()
        );

        // Send with retries
        match retry_with_backoff(self.config.max_retries, || self.send_batch(&batch)).await {
            Ok(()) => {
                info!(
                    "Successfully sent batch {} ({} changesets)",
                    batch.batch_id,
                    batch.changesets.len()
                );
                self.buffer_size = 0;
                self.last_flush = Instant::now();
                return;
            }
            Err(e) => {
                error!(
                    "Failed to send batch after {} attempts: {}",
                    self.config.max_retries + 1,
                    e
                );
            }
        }

        // If we failed after all retries, save to recovery storage for later retry
        if let Some(ref recovery) = self.recovery {
            warn!(
                "Saving {} changesets to recovery storage after failed send",
                batch.changesets.len()
            );
            for changeset in &batch.changesets {
                if let Err(e) = recovery.save_failed_changeset(changeset, "Max retries exceeded") {
                    error!("Failed to save changeset to recovery storage: {}", e);
                }
            }
        } else {
            error!(
                "Dropping {} changesets after failed send (recovery disabled)",
                batch.changesets.len()
            );
        }
    }

    async fn send_batch(&self, batch: &ChangesetBatch) -> Result<(), reqwest::Error> {
        let url = format!("{}/changesets", self.config.sequencer_url);

        self.client
            .post(&url)
            .json(batch)
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }

    /// Retry failed changesets from previous runs
    async fn retry_failed_changesets(&self) {
        let Some(ref recovery) = self.recovery else {
            return;
        };

        // Clean up old failures (older than 7 days)
        if let Err(e) = recovery.cleanup_old_failures(7) {
            warn!("Failed to clean up old recovery entries: {}", e);
        }

        // Log recovery status
        if let Ok((changeset_count, snapshot_count)) = recovery.get_failed_counts() {
            if changeset_count > 0 || snapshot_count > 0 {
                info!(
                    "Recovery storage contains {} changesets and {} snapshots",
                    changeset_count, snapshot_count
                );
            }
        }

        let failed = match recovery.get_failed_changesets() {
            Ok(changesets) => changesets,
            Err(e) => {
                error!("Failed to load failed changesets from recovery: {}", e);
                return;
            }
        };

        if failed.is_empty() {
            debug!("No failed changesets to retry");
            return;
        }

        info!(
            "Retrying {} failed changesets from previous runs",
            failed.len()
        );

        for (id, changeset) in failed {
            // Obtain fresh attestation token if configured
            let attestation_token = if let Some(ref attestation) = self.attestation {
                match attestation.get_token().await {
                    Ok(token) => Some(token),
                    Err(e) => {
                        warn!("Failed to obtain attestation token for retry: {}", e);
                        None
                    }
                }
            } else {
                None
            };

            let batch = ChangesetBatch {
                changesets: vec![changeset.clone()],
                batch_id: uuid::Uuid::new_v4().to_string(),
                attestation_token,
            };

            match retry_with_backoff(self.config.max_retries, || self.send_batch(&batch)).await {
                Ok(()) => {
                    info!(
                        "Successfully retried changeset at sequence {}",
                        changeset.sequence
                    );
                    if let Err(e) = recovery.remove_changeset(id) {
                        error!("Failed to remove retried changeset from recovery: {}", e);
                    }
                }
                Err(e) => {
                    error!(
                        "Failed to retry changeset at sequence {} after {} attempts: {}",
                        changeset.sequence,
                        self.config.max_retries + 1,
                        e
                    );
                    if let Err(e) = recovery.increment_changeset_retry(id, &e.to_string()) {
                        error!("Failed to update retry count: {}", e);
                    }
                }
            }
        }

        info!("Completed retry of failed changesets");
    }
}
