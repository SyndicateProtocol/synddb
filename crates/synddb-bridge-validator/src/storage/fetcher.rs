//! Storage fetcher for Witness Validators
//!
//! Fetches message records from various storage backends.

use anyhow::{Context, Result};
use tracing::debug;

use super::record::{MessageRecord, StorageRecord};

/// Message data fetched from storage
#[derive(Debug, Clone)]
pub struct FetchedMessage {
    pub message_type: String,
    pub calldata: Vec<u8>,
    pub metadata: serde_json::Value,
    pub metadata_hash: [u8; 32],
    pub nonce: u64,
    pub timestamp: u64,
    pub domain: [u8; 32],
    pub value: Option<u128>,
}

impl From<MessageRecord> for FetchedMessage {
    fn from(record: MessageRecord) -> Self {
        Self {
            message_type: record.message_type,
            calldata: record.calldata,
            metadata: record.metadata,
            metadata_hash: record.metadata_hash,
            nonce: record.nonce,
            timestamp: record.timestamp,
            domain: record.domain,
            value: None,
        }
    }
}

/// Fetches messages from storage backends
pub struct StorageFetcher {
    http_client: reqwest::Client,
    ipfs_gateway: String,
    arweave_gateway: String,
}

impl StorageFetcher {
    pub fn new() -> Self {
        Self {
            http_client: reqwest::Client::new(),
            ipfs_gateway: "https://ipfs.io/ipfs".to_string(),
            arweave_gateway: "https://arweave.net".to_string(),
        }
    }

    pub fn with_gateways(ipfs_gateway: String, arweave_gateway: String) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            ipfs_gateway,
            arweave_gateway,
        }
    }

    /// Fetch a message from storage
    pub async fn fetch(&self, storage_ref: &str) -> Result<FetchedMessage> {
        // Parse storage ref - can be:
        // - ipfs://QmHash
        // - ar://TxId
        // - https://... or http://...
        // - gcs://bucket/path
        // - memory://id (for testing)

        let url = self.resolve_url(storage_ref)?;
        debug!(url = %url, "Fetching from storage");

        let response = self
            .http_client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch from storage")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Storage fetch failed with status {}: {}",
                response.status(),
                response
                    .text()
                    .await
                    .unwrap_or_else(|_| "unknown error".to_string())
            );
        }

        let record: StorageRecord = response
            .json()
            .await
            .context("Failed to parse storage record")?;

        Ok(record.message.into())
    }

    fn resolve_url(&self, storage_ref: &str) -> Result<String> {
        if let Some(hash) = storage_ref.strip_prefix("ipfs://") {
            Ok(format!("{}/{}", self.ipfs_gateway, hash))
        } else if let Some(tx_id) = storage_ref.strip_prefix("ar://") {
            Ok(format!("{}/{}", self.arweave_gateway, tx_id))
        } else if storage_ref.starts_with("http://") || storage_ref.starts_with("https://") {
            Ok(storage_ref.to_string())
        } else if storage_ref.starts_with("gcs://") {
            // GCS URLs need to be converted to HTTP
            let path = storage_ref.strip_prefix("gcs://").unwrap();
            Ok(format!("https://storage.googleapis.com/{}", path))
        } else if storage_ref.starts_with("memory://") {
            // Memory storage is for testing - not actually fetchable
            anyhow::bail!("Memory storage refs cannot be fetched remotely")
        } else {
            anyhow::bail!("Unsupported storage ref scheme: {}", storage_ref)
        }
    }
}

impl Default for StorageFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_ipfs_url() {
        let fetcher = StorageFetcher::new();
        let url = fetcher.resolve_url("ipfs://QmTest123").unwrap();
        assert_eq!(url, "https://ipfs.io/ipfs/QmTest123");
    }

    #[test]
    fn test_resolve_arweave_url() {
        let fetcher = StorageFetcher::new();
        let url = fetcher.resolve_url("ar://abc123").unwrap();
        assert_eq!(url, "https://arweave.net/abc123");
    }

    #[test]
    fn test_resolve_https_url() {
        let fetcher = StorageFetcher::new();
        let url = fetcher.resolve_url("https://example.com/message.json").unwrap();
        assert_eq!(url, "https://example.com/message.json");
    }

    #[test]
    fn test_resolve_gcs_url() {
        let fetcher = StorageFetcher::new();
        let url = fetcher.resolve_url("gcs://my-bucket/path/to/message.json").unwrap();
        assert_eq!(url, "https://storage.googleapis.com/my-bucket/path/to/message.json");
    }

    #[test]
    fn test_resolve_memory_url_fails() {
        let fetcher = StorageFetcher::new();
        assert!(fetcher.resolve_url("memory://test-id").is_err());
    }
}
