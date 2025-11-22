//! Configuration types for the chain monitor.

use alloy::primitives::{Address, B256};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use url::Url;

/// Configuration for the chain monitor service.
///
/// This configuration specifies which blockchain to monitor, which contract
/// to watch, and how to connect to the RPC endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainMonitorConfig {
    /// List of WebSocket RPC URLs to connect to.
    ///
    /// The monitor will attempt to connect to these URLs in order,
    /// falling back to the next URL if a connection fails.
    ///
    /// Example: `["wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY"]`
    pub ws_urls: Vec<Url>,

    /// The address of the contract to monitor.
    ///
    /// All events from this contract will be monitored.
    pub contract_address: Address,

    /// The block number to start monitoring from.
    ///
    /// Set this to a recent block to avoid processing historical events,
    /// or to 0 to process all events from the beginning.
    pub start_block: u64,

    /// Optional event signature to filter for.
    ///
    /// If specified, only events with this signature will be processed.
    /// If `None`, all events from the contract will be processed.
    ///
    /// The event signature can be obtained from event definitions using
    /// `YourEvent::SIGNATURE_HASH`.
    pub event_signature: Option<B256>,

    /// Timeout for individual RPC requests (default: 10 seconds).
    #[serde(default = "default_request_timeout")]
    #[serde(with = "humantime_serde")]
    pub request_timeout: Duration,

    /// Timeout specifically for eth_getLogs requests (default: 5 minutes).
    ///
    /// This is typically longer than regular requests because getLogs
    /// can take a long time for large block ranges.
    #[serde(default = "default_get_logs_timeout")]
    #[serde(with = "humantime_serde")]
    pub get_logs_timeout: Duration,

    /// Size of the WebSocket subscription channel buffer (default: 1024).
    #[serde(default = "default_channel_size")]
    pub channel_size: usize,

    /// Duration to wait between retry attempts (default: 1 second).
    #[serde(default = "default_retry_interval")]
    #[serde(with = "humantime_serde")]
    pub retry_interval: Duration,

    /// Path to the SQLite database for event persistence.
    ///
    /// This database is used to track processed events and maintain
    /// idempotency across restarts.
    #[serde(default = "default_event_store_path")]
    pub event_store_path: String,
}

impl ChainMonitorConfig {
    /// Create a new configuration with the minimum required fields.
    ///
    /// Other fields will be set to sensible defaults.
    pub fn new(ws_urls: Vec<Url>, contract_address: Address, start_block: u64) -> Self {
        Self {
            ws_urls,
            contract_address,
            start_block,
            event_signature: None,
            request_timeout: default_request_timeout(),
            get_logs_timeout: default_get_logs_timeout(),
            channel_size: default_channel_size(),
            retry_interval: default_retry_interval(),
            event_store_path: default_event_store_path(),
        }
    }

    /// Set the event signature filter.
    pub fn with_event_signature(mut self, signature: B256) -> Self {
        self.event_signature = Some(signature);
        self
    }

    /// Set the event store database path.
    pub fn with_event_store_path(mut self, path: String) -> Self {
        self.event_store_path = path;
        self
    }

    /// Set the request timeout.
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Set the get_logs timeout.
    pub fn with_get_logs_timeout(mut self, timeout: Duration) -> Self {
        self.get_logs_timeout = timeout;
        self
    }

    /// Set the channel size for WebSocket subscriptions.
    pub fn with_channel_size(mut self, size: usize) -> Self {
        self.channel_size = size;
        self
    }

    /// Set the retry interval.
    pub fn with_retry_interval(mut self, interval: Duration) -> Self {
        self.retry_interval = interval;
        self
    }
}

fn default_request_timeout() -> Duration {
    Duration::from_secs(10)
}

fn default_get_logs_timeout() -> Duration {
    Duration::from_secs(300) // 5 minutes
}

fn default_channel_size() -> usize {
    1024
}

fn default_retry_interval() -> Duration {
    Duration::from_secs(1)
}

fn default_event_store_path() -> String {
    "./chain_events.db".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_new_sets_defaults() {
        let ws_url = Url::parse("wss://example.com").unwrap();
        let contract_addr = Address::from([0x42; 20]);
        let start_block = 12345;

        let config = ChainMonitorConfig::new(vec![ws_url.clone()], contract_addr, start_block);

        assert_eq!(config.ws_urls, vec![ws_url]);
        assert_eq!(config.contract_address, contract_addr);
        assert_eq!(config.start_block, start_block);
        assert_eq!(config.event_signature, None);
        assert_eq!(config.request_timeout, Duration::from_secs(10));
        assert_eq!(config.get_logs_timeout, Duration::from_secs(300));
        assert_eq!(config.channel_size, 1024);
        assert_eq!(config.retry_interval, Duration::from_secs(1));
        assert_eq!(config.event_store_path, "./chain_events.db");
    }

    #[test]
    fn test_config_builder_pattern() {
        let ws_url = Url::parse("wss://example.com").unwrap();
        let contract_addr = Address::from([0x42; 20]);
        let event_sig = B256::from([0x01; 32]);

        let config = ChainMonitorConfig::new(vec![ws_url], contract_addr, 100)
            .with_event_signature(event_sig)
            .with_event_store_path("/custom/path.db".to_string())
            .with_request_timeout(Duration::from_secs(20))
            .with_get_logs_timeout(Duration::from_secs(600))
            .with_channel_size(2048)
            .with_retry_interval(Duration::from_secs(5));

        assert_eq!(config.event_signature, Some(event_sig));
        assert_eq!(config.event_store_path, "/custom/path.db");
        assert_eq!(config.request_timeout, Duration::from_secs(20));
        assert_eq!(config.get_logs_timeout, Duration::from_secs(600));
        assert_eq!(config.channel_size, 2048);
        assert_eq!(config.retry_interval, Duration::from_secs(5));
    }

    #[test]
    fn test_config_serialization() {
        let ws_url = Url::parse("wss://base-mainnet.example.com/v2/key").unwrap();
        let contract_addr = Address::from([0xAB; 20]);

        let config = ChainMonitorConfig::new(vec![ws_url], contract_addr, 10_000_000);

        // Serialize to JSON
        let json = serde_json::to_string(&config).unwrap();

        // Deserialize back
        let deserialized: ChainMonitorConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config.ws_urls, deserialized.ws_urls);
        assert_eq!(config.contract_address, deserialized.contract_address);
        assert_eq!(config.start_block, deserialized.start_block);
        assert_eq!(config.request_timeout, deserialized.request_timeout);
        assert_eq!(config.get_logs_timeout, deserialized.get_logs_timeout);
    }

    #[test]
    fn test_config_yaml_serialization() {
        let ws_url = Url::parse("wss://base-mainnet.example.com").unwrap();
        let contract_addr = Address::from([0xCD; 20]);

        let config = ChainMonitorConfig::new(vec![ws_url], contract_addr, 5_000_000)
            .with_event_store_path("/data/events.db".to_string());

        // Serialize to YAML
        let yaml = serde_yaml::to_string(&config).unwrap();

        // Deserialize back
        let deserialized: ChainMonitorConfig = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(config.contract_address, deserialized.contract_address);
        assert_eq!(config.start_block, deserialized.start_block);
        assert_eq!(config.event_store_path, deserialized.event_store_path);
    }

    #[test]
    fn test_multiple_ws_urls() {
        let url1 = Url::parse("wss://primary.example.com").unwrap();
        let url2 = Url::parse("wss://backup.example.com").unwrap();
        let contract_addr = Address::from([0xEF; 20]);

        let config = ChainMonitorConfig::new(
            vec![url1.clone(), url2.clone()],
            contract_addr,
            1000,
        );

        assert_eq!(config.ws_urls.len(), 2);
        assert_eq!(config.ws_urls[0], url1);
        assert_eq!(config.ws_urls[1], url2);
    }
}
