//! Background sender that batches and sends changesets to sequencer

use crate::config::Config;
use crate::session::Changeset;
use crossbeam_channel::{select, Receiver};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tracing::{debug, error, info, warn};

#[derive(Debug, Serialize, Deserialize)]
struct ChangesetBatch {
    changesets: Vec<Changeset>,
    batch_id: String,
}

pub struct ChangesetSender {
    config: Config,
    client: Client,
    buffer: Vec<Changeset>,
    buffer_size: usize,
    last_flush: Instant,
}

impl ChangesetSender {
    pub fn new(config: Config) -> Self {
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
        }
    }

    pub async fn run(mut self, changeset_rx: Receiver<Changeset>, shutdown_rx: Receiver<()>) {
        info!("ChangesetSender started");

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

        let batch = ChangesetBatch {
            changesets: std::mem::take(&mut self.buffer),
            batch_id: uuid::Uuid::new_v4().to_string(),
        };

        debug!(
            "Flushing {} changesets to sequencer",
            batch.changesets.len()
        );

        // Send with retries
        for attempt in 0..=self.config.max_retries {
            match self.send_batch(&batch).await {
                Ok(_) => {
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
                    if attempt < self.config.max_retries {
                        warn!("Failed to send batch (attempt {}): {}", attempt + 1, e);
                        tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
                    } else {
                        error!("Failed to send batch after {} attempts: {}", attempt + 1, e);
                        // TODO: Consider persisting failed batches to disk
                    }
                }
            }
        }

        // If we failed, put changesets back (or drop them)
        // For now we'll drop them to avoid unbounded memory growth
        error!(
            "Dropping {} changesets after failed send",
            batch.changesets.len()
        );
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
}
