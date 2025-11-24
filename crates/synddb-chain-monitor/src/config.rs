//! Configuration types for the chain monitor.

use alloy::primitives::{Address, B256};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use url::Url;

/// Configuration for the chain monitor service.
///
/// This configuration specifies which blockchain to monitor, which contract
/// to watch, and how to connect to the RPC endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, Parser, Default)]
#[command(author, version, about, long_about = None)]
pub struct ChainMonitorConfig {
    /// WebSocket RPC URL (can be specified multiple times for failover)
    #[arg(long, env = "WS_URL", value_parser = parse_url, required = true)]
    pub ws_urls: Vec<Url>,

    /// Contract address to monitor
    #[arg(long, env = "CONTRACT_ADDRESS", value_parser = parse_address, required = true)]
    pub contract_address: Address,

    /// Block number to start monitoring from
    #[arg(long, env = "START_BLOCK", required = true)]
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
    #[arg(long, env = "REQUEST_TIMEOUT", default_value = "10s", value_parser = parse_duration)]
    #[serde(with = "humantime_serde")]
    pub request_timeout: Duration,

    /// Timeout for `eth_getLogs` requests (e.g., "5m")
    #[arg(long, env = "GET_LOGS_TIMEOUT", default_value = "300s", value_parser = parse_duration)]
    #[serde(with = "humantime_serde")]
    pub get_logs_timeout: Duration,

    /// Size of the WebSocket subscription channel buffer
    #[arg(long, env = "CHANNEL_SIZE", default_value = "1024")]
    pub channel_size: usize,

    /// Duration to wait between retry attempts (e.g., "1s")
    #[arg(long, env = "RETRY_INTERVAL", default_value = "1s", value_parser = parse_duration)]
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
    /// Other fields will be set to sensible defaults.
    pub fn new(ws_urls: Vec<Url>, contract_address: Address, start_block: u64) -> Self {
        Self {
            ws_urls,
            contract_address,
            start_block,
            ..Default::default()
        }
    }

    /// Set the event signature filter.
    pub const fn with_event_signature(mut self, signature: B256) -> Self {
        self.event_signature = Some(signature);
        self
    }

    /// Set the event store database path.
    pub fn with_event_store_path(mut self, path: impl Into<String>) -> Self {
        self.event_store_path = path.into();
        self
    }

    /// Set the request timeout.
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Set the `get_logs` timeout.
    pub const fn with_get_logs_timeout(mut self, timeout: Duration) -> Self {
        self.get_logs_timeout = timeout;
        self
    }

    /// Set the channel size for WebSocket subscriptions.
    pub const fn with_channel_size(mut self, size: usize) -> Self {
        self.channel_size = size;
        self
    }

    /// Set the retry interval.
    pub const fn with_retry_interval(mut self, interval: Duration) -> Self {
        self.retry_interval = interval;
        self
    }
}

// TODO: refactor to a `/shared` dir later
// Custom parsers for clap
fn parse_url(s: &str) -> Result<Url, String> {
    Url::parse(s).map_err(|e| format!("Invalid URL: {}", e))
}

fn parse_address(s: &str) -> Result<Address, String> {
    s.parse().map_err(|e| format!("Invalid address: {}", e))
}

fn parse_b256(s: &str) -> Result<B256, String> {
    // First try parsing as hex string
    if let Ok(hash) = s.parse::<B256>() {
        return Ok(hash);
    }

    // If that fails, try parsing as Solidity event definition
    // e.g., "event Deposit(address indexed from, uint256 amount)"
    parse_event_signature_from_definition(s)
}

/// Parse a Solidity event definition and compute its signature hash.
///
/// Supports formats like:
/// - "event Deposit(address indexed from, uint256 amount)"
/// - "Deposit(address,uint256)" (canonical signature)
///
/// Returns the keccak256 hash of the canonical event signature.
fn parse_event_signature_from_definition(s: &str) -> Result<B256, String> {
    let s = s.trim();

    // Extract the event signature (name and parameter types)
    let signature = if s.starts_with("event ") {
        // Full event definition: "event Deposit(address indexed from, uint256 amount)"
        extract_canonical_signature(s).ok_or_else(|| format!("Invalid event definition: {}", s))?
    } else if s.contains('(') && s.contains(')') {
        // Already in canonical form: "Deposit(address,uint256)"
        s.to_string()
    } else {
        return Err(
            "Invalid format. Expected hex string (0x...) or event definition (event Name(...))"
                .into(),
        );
    };

    // Compute keccak256 hash of the canonical signature
    use alloy::primitives::keccak256;
    let hash = keccak256(signature.as_bytes());

    Ok(hash)
}

/// Extract canonical event signature from full Solidity event definition.
///
/// Example: "event Deposit(address indexed from, uint256 amount)"
/// Returns: "Deposit(address,uint256)"
fn extract_canonical_signature(event_def: &str) -> Option<String> {
    // Remove "event " prefix
    let event_def = event_def.strip_prefix("event ")?.trim();

    // Find the event name and parameters
    let paren_start = event_def.find('(')?;
    let paren_end = event_def.rfind(')')?;

    let name = event_def[..paren_start].trim();
    let params = &event_def[paren_start + 1..paren_end];

    // Extract just the types (removing "indexed" and parameter names)
    let types: Vec<&str> = params
        .split(',')
        .filter_map(|param| {
            let param = param.trim();
            if param.is_empty() {
                return None;
            }

            // Split by whitespace and take the first part (the type)
            let parts: Vec<&str> = param.split_whitespace().collect();
            if parts.is_empty() {
                return None;
            }

            // Skip "indexed" keyword and take the type
            let type_part = if parts[0] == "indexed" {
                parts.get(1)?
            } else {
                parts[0]
            };

            Some(type_part)
        })
        .collect();

    Some(format!("{}({})", name, types.join(",")))
}

fn parse_duration(s: &str) -> Result<Duration, String> {
    humantime::parse_duration(s).map_err(|e| format!("Invalid duration: {}", e))
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

        let mut config = ChainMonitorConfig::new(vec![ws_url], contract_addr, 5_000_000);
        config.event_store_path = "/data/events.db".to_string();

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
    fn test_extract_canonical_signature() {
        assert_eq!(
            extract_canonical_signature("event Deposit(address indexed from, uint256 amount)"),
            Some("Deposit(address,uint256)".to_string())
        );

        assert_eq!(
            extract_canonical_signature("event Transfer(address from, address to, uint256 value)"),
            Some("Transfer(address,address,uint256)".to_string())
        );

        assert_eq!(
            extract_canonical_signature(
                "event Approval(address indexed owner, address indexed spender, uint256 value)"
            ),
            Some("Approval(address,address,uint256)".to_string())
        );
    }

    #[test]
    fn test_parse_event_signature_invalid() {
        // Test invalid inputs
        assert!(parse_b256("not a valid signature").is_err());
        assert!(parse_b256("event NoParentheses").is_err());
    }
}
