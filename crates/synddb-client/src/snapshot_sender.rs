//! Background sender for database snapshots to sequencer

use crate::{
    attestation::AttestationClient, config::Config, recovery::FailedBatchRecovery,
    retry::retry_with_backoff, sender::SendError, session::Snapshot,
};
use crossbeam_channel::{select, Receiver};
use reqwest::Client;
use std::sync::Arc;
use synddb_shared::types::payloads::{SnapshotData, SnapshotRequest};
use tracing::{debug, error, info, warn};

impl From<&Snapshot> for SnapshotData {
    fn from(snap: &Snapshot) -> Self {
        Self {
            data: snap.data.clone(),
            timestamp: snap
                .timestamp
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            sequence: snap.sequence,
        }
    }
}

#[derive(Debug)]
pub struct SnapshotSender {
    config: Config,
    client: Client,
    recovery: Option<Arc<FailedBatchRecovery>>,
    attestation: Option<AttestationClient>,
}

impl SnapshotSender {
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
            recovery,
            attestation,
        }
    }

    pub(crate) async fn run(self, snapshot_rx: Receiver<Snapshot>, shutdown_rx: Receiver<()>) {
        info!("SnapshotSender started");

        // Retry any failed snapshots from previous runs
        self.retry_failed_snapshots().await;

        loop {
            select! {
                recv(snapshot_rx) -> snapshot => {
                    match snapshot {
                        Ok(snapshot) => {
                            info!(
                                "Received snapshot: {} bytes at sequence {}",
                                snapshot.data.len(),
                                snapshot.sequence
                            );
                            self.send_snapshot(snapshot).await;
                        }
                        Err(_) => {
                            warn!("Snapshot channel closed");
                            break;
                        }
                    }
                }
                recv(shutdown_rx) -> _ => {
                    info!("Shutdown signal received");
                    break;
                }
            }
        }

        info!("SnapshotSender stopped");
    }

    async fn send_snapshot(&self, snapshot: Snapshot) {
        // Obtain attestation token if configured
        let attestation_token = if let Some(ref attestation) = self.attestation {
            match attestation.get_token().await {
                Ok(token) => {
                    debug!("Obtained attestation token for snapshot");
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

        let message_id = uuid::Uuid::new_v4().to_string();
        let request = SnapshotRequest {
            snapshot: SnapshotData::from(&snapshot),
            message_id: message_id.clone(),
            attestation_token,
        };

        debug!(
            "Sending snapshot to sequencer (seq={}, size={} bytes, attestation: {})",
            snapshot.sequence,
            snapshot.data.len(),
            request.attestation_token.is_some()
        );

        // Send with retries
        match retry_with_backoff("send_snapshot", self.config.max_retries, || {
            self.send_snapshot_internal(&request)
        })
        .await
        {
            Ok(()) => {
                info!(
                    "Successfully sent snapshot {} (sequence {})",
                    message_id, snapshot.sequence
                );
                return;
            }
            Err(e) => {
                error!(
                    "Failed to send snapshot after {} attempts: {}",
                    self.config.max_retries, e
                );
            }
        }

        // If we failed after all retries, save to recovery storage for later retry
        if let Some(ref recovery) = self.recovery {
            warn!(
                "Saving snapshot at sequence {} to recovery storage after failed send",
                snapshot.sequence
            );
            if let Err(e) = recovery.save_failed_snapshot(&snapshot, "Max retries exceeded") {
                error!("Failed to save snapshot to recovery storage: {}", e);
            }
        } else {
            error!(
                "Dropping snapshot at sequence {} after failed send (recovery disabled)",
                snapshot.sequence
            );
        }
    }

    async fn send_snapshot_internal(&self, request: &SnapshotRequest) -> Result<(), SendError> {
        let url = self
            .config
            .sequencer_url
            .join("snapshots")
            .expect("valid URL path");

        // Serialize to CBOR
        let cbor_bytes = request.to_cbor().map_err(SendError::Cbor)?;

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

    /// Retry failed snapshots from previous runs
    async fn retry_failed_snapshots(&self) {
        let Some(ref recovery) = self.recovery else {
            return;
        };

        let failed = match recovery.get_failed_snapshots() {
            Ok(snapshots) => snapshots,
            Err(e) => {
                error!("Failed to load failed snapshots from recovery: {}", e);
                return;
            }
        };

        if failed.is_empty() {
            debug!("No failed snapshots to retry");
            return;
        }

        info!(
            "Retrying {} failed snapshots from previous runs",
            failed.len()
        );

        for (id, snapshot) in failed {
            // Obtain fresh attestation token if configured
            let attestation_token = if let Some(ref attestation) = self.attestation {
                match attestation.get_token().await {
                    Ok(token) => Some(token),
                    Err(e) => {
                        warn!(
                            "Failed to obtain attestation token for snapshot retry: {}",
                            e
                        );
                        None
                    }
                }
            } else {
                None
            };

            let request = SnapshotRequest {
                snapshot: SnapshotData::from(&snapshot),
                message_id: uuid::Uuid::new_v4().to_string(),
                attestation_token,
            };

            match retry_with_backoff("retry_failed_snapshot", self.config.max_retries, || {
                self.send_snapshot_internal(&request)
            })
            .await
            {
                Ok(()) => {
                    info!(
                        "Successfully retried snapshot at sequence {}",
                        snapshot.sequence
                    );
                    if let Err(e) = recovery.remove_snapshot(id) {
                        error!("Failed to remove retried snapshot from recovery: {}", e);
                    }
                }
                Err(e) => {
                    error!(
                        "Failed to retry snapshot at sequence {} after {} attempts: {}",
                        snapshot.sequence, self.config.max_retries, e
                    );
                    if let Err(e) = recovery.increment_snapshot_retry(id, &e.to_string()) {
                        error!("Failed to update snapshot retry count: {}", e);
                    }
                }
            }
        }

        info!("Completed retry of failed snapshots");
    }
}
