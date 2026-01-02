//! Configuration types for the chain monitor.

use alloy::primitives::{Address, B256};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use synddb_shared::parse::{parse_address, parse_b256, parse_url};
use url::Url;

/// Configuration for the chain monitor service.
///
/// This configuration specifies which blockchain to monitor, which contract
/// to watch, and how to connect to the RPC endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
#[command(author, version, about, long_about = None)]
pub struct ChainMonitorConfig {
    /// WebSocket RPC URL (can be specified multiple times for failover)
    ///
    /// Default is Anvil's local WebSocket endpoint.
    #[arg(long, env = "WS_URL", value_parser = parse_url, default_value = "ws://localhost:8545")]
    pub ws_urls: Vec<Url>,

    /// Contract address to monitor
    ///
    /// Default is the zero address (placeholder). You must set this to your
    /// deployed contract address for chain monitoring to work.
    #[arg(long, env = "CONTRACT_ADDRESS", value_parser = parse_address, default_value = "0x0000000000000000000000000000000000000000")]
    pub contract_address: Address,

    // TODO - automatically get this from Bridge deployment metadata
    /// Block number to start monitoring from. This should be the block of the Bridge contract deployment.
    #[arg(long, env = "START_BLOCK", default_value_t = 1)]
    pub start_block: u64,

    /// Optional event signature to filter for
    ///
    /// Accepts multiple formats:
    /// - Hex string: "0x7fcf532c15f0a6db0bd6d0e038bea71d30d808c7d98cb3bf7268a95bf5081b65"
    /// - Solidity event definition: "event Deposit(address indexed from, uint256 amount)"
    /// - Canonical signature: "Deposit(address,uint256)"
    #[arg(long, env = "EVENT_SIGNATURE", value_parser = parse_b256)]
    #[serde(default)]
    pub event_signature: Option<B256>,

    /// Timeout for individual RPC requests (e.g., "10s")
    #[arg(long, env = "MONITOR_REQUEST_TIMEOUT", default_value = "10s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub monitor_request_timeout: Duration,

    /// Timeout for `eth_getLogs` requests (e.g., "5m")
    #[arg(long, env = "GET_LOGS_TIMEOUT", default_value = "300s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub get_logs_timeout: Duration,

    /// Size of the WebSocket subscription channel buffer
    #[arg(long, env = "CHANNEL_SIZE", default_value = "1024")]
    pub channel_size: usize,

    /// Duration to wait between retry attempts (e.g., "1s")
    #[arg(long, env = "RETRY_INTERVAL", default_value = "1s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub retry_interval: Duration,

    /// Path to `SQLite` database for event persistence
    /// NOTE: This database is separate from the application's `SQLite` database
    #[arg(long, env = "EVENT_STORE_PATH", default_value = "./chain_events.db")]
    pub event_store_path: String,
}

impl ChainMonitorConfig {
    /// Create a new configuration with the minimum required fields.
    ///
    /// Other fields will be set to sensible defaults from clap's `default_value` attributes.
    pub fn new(ws_urls: Vec<Url>, contract_address: Address, start_block: u64) -> Self {
        // Use parse_from with minimal args to get defaults from clap's `default_value`
        let mut config = Self::parse_from([
            "chain-monitor",
            "--ws-urls",
            "ws://placeholder.invalid",
            "--contract-address",
            "0x0000000000000000000000000000000000000000",
            "--start-block",
            "0",
        ]);
        // Override with actual values
        config.ws_urls = ws_urls;
        config.contract_address = contract_address;
        config.start_block = start_block;
        config
    }

    /// Set the event signature filter.
    #[must_use]
    pub const fn with_event_signature(mut self, signature: B256) -> Self {
        self.event_signature = Some(signature);
        self
    }

    /// Set the event store database path.
    #[must_use]
    pub fn with_event_store_path(mut self, path: impl Into<String>) -> Self {
        self.event_store_path = path.into();
        self
    }

    /// Set the request timeout.
    #[must_use]
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.monitor_request_timeout = timeout;
        self
    }

    /// Set the `get_logs` timeout.
    #[must_use]
    pub const fn with_get_logs_timeout(mut self, timeout: Duration) -> Self {
        self.get_logs_timeout = timeout;
        self
    }

    /// Set the channel size for WebSocket subscriptions.
    #[must_use]
    pub const fn with_channel_size(mut self, size: usize) -> Self {
        self.channel_size = size;
        self
    }

    /// Set the retry interval.
    #[must_use]
    pub const fn with_retry_interval(mut self, interval: Duration) -> Self {
        self.retry_interval = interval;
        self
    }
}

const fn default_request_timeout() -> Duration {
    Duration::from_secs(10)
}

const fn default_get_logs_timeout() -> Duration {
    Duration::from_secs(300) // 5 minutes
}

const fn default_channel_size() -> usize {
    1024
}

const fn default_retry_interval() -> Duration {
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
        assert_eq!(config.monitor_request_timeout, Duration::from_secs(10));
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
        assert_eq!(config.monitor_request_timeout, Duration::from_secs(20));
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
        assert_eq!(
            config.monitor_request_timeout,
            deserialized.monitor_request_timeout
        );
        assert_eq!(config.get_logs_timeout, deserialized.get_logs_timeout);
    }

    #[test]
    fn test_config_yaml_serialization() {
        let ws_url = Url::parse("wss://base-mainnet.example.com").unwrap();
        let contract_addr = Address::from([0xCD; 20]);

        let mut config = ChainMonitorConfig::new(vec![ws_url], contract_addr, 5_000_000);
        config.event_store_path = "/data/events.db".to_string();

        // Serialize to YAML
        let yaml = serde_saphyr::to_string(&config).unwrap();

        // Deserialize back
        let deserialized: ChainMonitorConfig = serde_saphyr::from_str(&yaml).unwrap();

        assert_eq!(config.contract_address, deserialized.contract_address);
        assert_eq!(config.start_block, deserialized.start_block);
        assert_eq!(config.event_store_path, deserialized.event_store_path);
    }

    #[test]
    fn test_multiple_ws_urls() {
        let url1 = Url::parse("wss://primary.example.com").unwrap();
        let url2 = Url::parse("wss://backup.example.com").unwrap();
        let contract_addr = Address::from([0xEF; 20]);

        let config = ChainMonitorConfig::new(vec![url1.clone(), url2.clone()], contract_addr, 1000);

        assert_eq!(config.ws_urls.len(), 2);
        assert_eq!(config.ws_urls[0], url1);
        assert_eq!(config.ws_urls[1], url2);
    }

    #[test]
    fn test_parse_event_signature_hex() {
        // Test parsing hex string directly
        let hex = "0x7fcf532c15f0a6db0bd6d0e038bea71d30d808c7d98cb3bf7268a95bf5081b65";
        let result = parse_b256(hex);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_event_signature_from_full_definition() {
        // Test parsing full Solidity event definition
        let event_def = "event Deposit(address indexed from, address indexed to, uint256 amount)";
        let result = parse_b256(event_def);
        assert!(result.is_ok());

        // Verify it matches the expected signature hash
        let expected = alloy::primitives::keccak256(b"Deposit(address,address,uint256)");
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_event_signature_from_canonical_form() {
        // Test parsing canonical signature (without "event" keyword and parameter names)
        let canonical = "Deposit(address,uint256)";
        let result = parse_b256(canonical);
        assert!(result.is_ok());

        let expected = alloy::primitives::keccak256(b"Deposit(address,uint256)");
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_event_signature_with_complex_types() {
        // Test with arrays and structs
        let event_def =
            "event Transfer(address indexed from, address indexed to, uint256[] amounts)";
        let result = parse_b256(event_def);
        assert!(result.is_ok());

        let expected =
            alloy::primitives::keccak256("Transfer(address,address,uint256[])".as_bytes());
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_event_signature_no_indexed() {
        // Test event without indexed parameters
        let event_def = "event StateSync(uint256 nonce, bytes data)";
        let result = parse_b256(event_def);
        assert!(result.is_ok());

        let expected = alloy::primitives::keccak256(b"StateSync(uint256,bytes)");
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_event_signature_invalid() {
        // Test invalid inputs
        assert!(parse_b256("not a valid signature").is_err());
        assert!(parse_b256("event NoParentheses").is_err());
    }
}
