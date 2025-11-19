//! Background sender for database snapshots to sequencer

use crate::config::Config;
use crate::persistence::FailedBatchPersistence;
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
}

pub struct SnapshotSender {
    config: Config,
    client: Client,
    persistence: Option<FailedBatchPersistence>,
}

impl SnapshotSender {
    pub fn new(config: Config, persistence_path: Option<PathBuf>) -> Self {
        let client = Client::builder()
            .timeout(config.request_timeout)
            .build()
            .expect("Failed to create HTTP client");

        let persistence =
            persistence_path.and_then(|path| match FailedBatchPersistence::new(path) {
                Ok(p) => Some(p),
                Err(e) => {
                    error!("Failed to initialize persistence: {}", e);
                    None
                }
            });

        Self {
            config,
            client,
            persistence,
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
        let message = SnapshotMessage {
            snapshot,
            message_id: uuid::Uuid::new_v4().to_string(),
        };

        debug!(
            "Sending snapshot to sequencer (seq={}, size={} bytes)",
            message.snapshot.sequence,
            message.snapshot.data.len()
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

        // If we failed after all retries, persist for later retry
        if let Some(ref persistence) = self.persistence {
            warn!(
                "Persisting snapshot at sequence {} after failed send",
                message.snapshot.sequence
            );
            if let Err(e) =
                persistence.save_failed_snapshot(&message.snapshot, "Max retries exceeded")
            {
                error!("Failed to persist snapshot: {}", e);
            }
        } else {
            error!(
                "Dropping snapshot at sequence {} after failed send (persistence disabled)",
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
