//! Background sender for database snapshots to sequencer

use crate::attestation::AttestationClient;
use crate::config::Config;
use crate::recovery::FailedBatchRecovery;
use crate::session::Snapshot;
use crossbeam_channel::{select, Receiver};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{debug, error, info, warn};

#[derive(Debug, Serialize, Deserialize)]
struct SnapshotMessage {
    snapshot: Snapshot,
    message_id: String,
    /// Optional TEE attestation token (JWT) proving workload identity
    #[serde(skip_serializing_if = "Option::is_none")]
    attestation_token: Option<String>,
}

pub struct SnapshotSender {
    config: Config,
    client: Client,
    recovery: Option<FailedBatchRecovery>,
    attestation: Option<AttestationClient>,
}

impl SnapshotSender {
    pub fn new(
        config: Config,
        recovery_path: Option<PathBuf>,
        attestation: Option<AttestationClient>,
    ) -> Self {
        let client = Client::builder()
            .timeout(config.request_timeout)
            .build()
            .expect("Failed to create HTTP client");

        let recovery = recovery_path.and_then(|path| match FailedBatchRecovery::new(path) {
            Ok(p) => Some(p),
            Err(e) => {
                error!("Failed to initialize recovery storage: {}", e);
                None
            }
        });

        Self {
            config,
            client,
            recovery,
            attestation,
        }
    }

    pub async fn run(self, snapshot_rx: Receiver<Snapshot>, shutdown_rx: Receiver<()>) {
        info!("SnapshotSender started");

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

        let message = SnapshotMessage {
            snapshot,
            message_id: uuid::Uuid::new_v4().to_string(),
            attestation_token,
        };

        debug!(
            "Sending snapshot to sequencer (seq={}, size={} bytes, attestation: {})",
            message.snapshot.sequence,
            message.snapshot.data.len(),
            message.attestation_token.is_some()
        );

        // Send with retries
        for attempt in 0..=self.config.max_retries {
            match self.send_snapshot_internal(&message).await {
                Ok(_) => {
                    info!(
                        "Successfully sent snapshot {} (sequence {})",
                        message.message_id, message.snapshot.sequence
                    );
                    return;
                }
                Err(e) => {
                    if attempt < self.config.max_retries {
                        warn!("Failed to send snapshot (attempt {}): {}", attempt + 1, e);
                        tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
                    } else {
                        error!(
                            "Failed to send snapshot after {} attempts: {}",
                            attempt + 1,
                            e
                        );
                        // TODO: Consider persisting failed snapshots to disk
                        // Snapshots are critical - losing them means validators may not be able to sync
                    }
                }
            }
        }

        // If we failed after all retries, save to recovery storage for later retry
        if let Some(ref recovery) = self.recovery {
            warn!(
                "Saving snapshot at sequence {} to recovery storage after failed send",
                message.snapshot.sequence
            );
            if let Err(e) = recovery.save_failed_snapshot(&message.snapshot, "Max retries exceeded")
            {
                error!("Failed to save snapshot to recovery storage: {}", e);
            }
        } else {
            error!(
                "Dropping snapshot at sequence {} after failed send (recovery disabled)",
                message.snapshot.sequence
            );
        }
    }

    async fn send_snapshot_internal(
        &self,
        message: &SnapshotMessage,
    ) -> Result<(), reqwest::Error> {
        let url = format!("{}/snapshots", self.config.sequencer_url);

        self.client
            .post(&url)
            .json(message)
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }
}
