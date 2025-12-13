//! Background sender that batches and sends changesets to sequencer

use crate::{
    attestation::AttestationClient, config::Config, recovery::FailedBatchRecovery,
    retry::retry_with_backoff, session::Changeset,
};
use crossbeam_channel::{select, Receiver};
use reqwest::Client;
use std::{fmt, sync::Arc, time::Instant};
use synddb_shared::types::payloads::{ChangesetBatchRequest, ChangesetData};
use tracing::{debug, error, info, warn};

/// Error type for send operations
#[derive(Debug)]
pub enum SendError {
    Http(reqwest::Error),
    Cbor(ciborium::ser::Error<std::io::Error>),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(e) => write!(f, "HTTP error: {e}"),
            Self::Cbor(e) => write!(f, "CBOR serialization error: {e}"),
        }
    }
}

impl std::error::Error for SendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(e) => Some(e),
            Self::Cbor(e) => Some(e),
        }
    }
}

impl From<&Changeset> for ChangesetData {
    fn from(cs: &Changeset) -> Self {
        Self {
            data: cs.data.clone(),
            sequence: cs.sequence,
            timestamp: cs
                .timestamp
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }
    }
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
            .timeout(config.snapshot_request_timeout)
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
            || self.last_flush.elapsed() >= self.config.flush_interval
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

        // Take changesets from buffer and convert to API format
        let changesets_raw = std::mem::take(&mut self.buffer);
        let changesets: Vec<ChangesetData> =
            changesets_raw.iter().map(ChangesetData::from).collect();
        let batch_id = uuid::Uuid::new_v4().to_string();

        let batch = ChangesetBatchRequest {
            batch_id: batch_id.clone(),
            changesets,
            attestation_token,
        };

        debug!(
            "Flushing {} changesets to sequencer (attestation: {})",
            batch.changesets.len(),
            batch.attestation_token.is_some()
        );

        // Send with retries
        match retry_with_backoff("send_changeset_batch", self.config.max_retries, || {
            self.send_batch(&batch)
        })
        .await
        {
            Ok(()) => {
                info!(
                    "Successfully sent batch {} ({} changesets)",
                    batch_id,
                    batch.changesets.len()
                );
                self.buffer_size = 0;
                self.last_flush = Instant::now();
                return;
            }
            Err(e) => {
                error!(
                    "Failed to send batch after {} attempts: {}",
                    self.config.max_retries, e
                );
            }
        }

        // If we failed after all retries, save to recovery storage for later retry
        if let Some(ref recovery) = self.recovery {
            warn!(
                "Saving {} changesets to recovery storage after failed send",
                changesets_raw.len()
            );
            for changeset in &changesets_raw {
                if let Err(e) = recovery.save_failed_changeset(changeset, "Max retries exceeded") {
                    error!("Failed to save changeset to recovery storage: {}", e);
                }
            }
        } else {
            error!(
                "Dropping {} changesets after failed send (recovery disabled)",
                changesets_raw.len()
            );
        }
    }

    async fn send_batch(&self, batch: &ChangesetBatchRequest) -> Result<(), SendError> {
        let url = self
            .config
            .sequencer_url
            .join("changesets")
            .expect("valid URL path");

        // Serialize to CBOR
        let cbor_bytes = batch.to_cbor().map_err(SendError::Cbor)?;

        self.client
            .post(url)
            .header("Content-Type", "application/cbor")
            .body(cbor_bytes)
            .send()
            .await
            .map_err(SendError::Http)?
            .error_for_status()
            .map_err(SendError::Http)?;

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

            let batch = ChangesetBatchRequest {
                batch_id: uuid::Uuid::new_v4().to_string(),
                changesets: vec![ChangesetData::from(&changeset)],
                attestation_token,
            };

            match retry_with_backoff("retry_failed_changeset", self.config.max_retries, || {
                self.send_batch(&batch)
            })
            .await
            {
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
                        changeset.sequence, self.config.max_retries, e
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
