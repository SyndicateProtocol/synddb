use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use sha3::{Digest, Keccak256};

pub struct SchemaFetcher {
    cache: RwLock<HashMap<String, CachedFetch>>,
    ttl: Duration,
    ipfs_gateway: String,
    arweave_gateway: String,
}

struct CachedFetch {
    schema: serde_json::Value,
    cached_at: Instant,
}

impl SchemaFetcher {
    pub fn new(ttl: Duration) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            ttl,
            ipfs_gateway: "https://ipfs.io/ipfs".to_string(),
            arweave_gateway: "https://arweave.net".to_string(),
        }
    }

    pub fn with_gateways(
        ttl: Duration,
        ipfs_gateway: String,
        arweave_gateway: String,
    ) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            ttl,
            ipfs_gateway,
            arweave_gateway,
        }
    }

    pub async fn fetch(
        &self,
        schema_uri: &str,
        expected_hash: Option<&[u8; 32]>,
    ) -> Result<serde_json::Value> {
        if schema_uri.is_empty() {
            anyhow::bail!("Empty schema URI");
        }

        // Check cache first
        {
            let cache = self.cache.read().unwrap();
            if let Some(cached) = cache.get(schema_uri) {
                if cached.cached_at.elapsed() < self.ttl {
                    return Ok(cached.schema.clone());
                }
            }
        }

        // Resolve URI to HTTP URL
        let http_url = self.resolve_uri(schema_uri)?;

        // Fetch from HTTP
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        let response = client
            .get(&http_url)
            .send()
            .await
            .context("Failed to fetch schema")?;

        if !response.status().is_success() {
            anyhow::bail!("Schema fetch failed with status: {}", response.status());
        }

        let body = response
            .bytes()
            .await
            .context("Failed to read schema body")?;

        // Verify hash if provided
        if let Some(expected) = expected_hash {
            let actual: [u8; 32] = Keccak256::digest(&body).into();
            if &actual != expected {
                anyhow::bail!(
                    "Schema hash mismatch: expected {}, got {}",
                    hex::encode(expected),
                    hex::encode(actual)
                );
            }
        }

        // Parse JSON
        let schema: serde_json::Value =
            serde_json::from_slice(&body).context("Failed to parse schema as JSON")?;

        // Update cache
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(
                schema_uri.to_string(),
                CachedFetch {
                    schema: schema.clone(),
                    cached_at: Instant::now(),
                },
            );
        }

        Ok(schema)
    }

    fn resolve_uri(&self, uri: &str) -> Result<String> {
        if uri.starts_with("ipfs://") {
            let hash = uri.strip_prefix("ipfs://").unwrap();
            Ok(format!("{}/{}", self.ipfs_gateway, hash))
        } else if uri.starts_with("ar://") {
            let tx_id = uri.strip_prefix("ar://").unwrap();
            Ok(format!("{}/{}", self.arweave_gateway, tx_id))
        } else if uri.starts_with("http://") || uri.starts_with("https://") {
            Ok(uri.to_string())
        } else {
            anyhow::bail!("Unsupported schema URI scheme: {}", uri)
        }
    }

    pub fn invalidate(&self, schema_uri: &str) {
        let mut cache = self.cache.write().unwrap();
        cache.remove(schema_uri);
    }

    pub fn clear_cache(&self) {
        let mut cache = self.cache.write().unwrap();
        cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_ipfs_uri() {
        let fetcher = SchemaFetcher::new(Duration::from_secs(3600));
        let url = fetcher
            .resolve_uri("ipfs://QmTest123456")
            .unwrap();
        assert_eq!(url, "https://ipfs.io/ipfs/QmTest123456");
    }

    #[test]
    fn test_resolve_arweave_uri() {
        let fetcher = SchemaFetcher::new(Duration::from_secs(3600));
        let url = fetcher.resolve_uri("ar://abc123xyz").unwrap();
        assert_eq!(url, "https://arweave.net/abc123xyz");
    }

    #[test]
    fn test_resolve_http_uri() {
        let fetcher = SchemaFetcher::new(Duration::from_secs(3600));
        let url = fetcher
            .resolve_uri("https://example.com/schema.json")
            .unwrap();
        assert_eq!(url, "https://example.com/schema.json");
    }

    #[test]
    fn test_unsupported_scheme() {
        let fetcher = SchemaFetcher::new(Duration::from_secs(3600));
        assert!(fetcher.resolve_uri("ftp://example.com").is_err());
    }

    #[test]
    fn test_custom_gateways() {
        let fetcher = SchemaFetcher::with_gateways(
            Duration::from_secs(3600),
            "https://custom-ipfs.example.com".to_string(),
            "https://custom-ar.example.com".to_string(),
        );

        let ipfs_url = fetcher.resolve_uri("ipfs://QmTest").unwrap();
        assert_eq!(ipfs_url, "https://custom-ipfs.example.com/QmTest");

        let ar_url = fetcher.resolve_uri("ar://test").unwrap();
        assert_eq!(ar_url, "https://custom-ar.example.com/test");
    }
}
