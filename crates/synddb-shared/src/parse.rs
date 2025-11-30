//! Common parsing utilities for clap and configuration
//!
//! These functions are designed to be used with clap's `value_parser` attribute
//! to parse command-line arguments and environment variables.

use alloy::primitives::{Address, B256};
use url::Url;

/// Parse a URL from a string
///
/// # Examples
/// ```
/// use synddb_shared::parse::parse_url;
///
/// assert!(parse_url("https://example.com").is_ok());
/// assert!(parse_url("invalid url").is_err());
/// ```
pub fn parse_url(s: &str) -> Result<Url, String> {
    Url::parse(s).map_err(|e| format!("Invalid URL: {}", e))
}

/// Parse an Ethereum address from a string
///
/// # Examples
/// ```
/// use synddb_shared::parse::parse_address;
///
/// let addr = parse_address("0x742d35Cc6634C0532925a3b844Bc454e4438f44e").unwrap();
/// ```
pub fn parse_address(s: &str) -> Result<Address, String> {
    s.parse().map_err(|e| format!("Invalid address: {}", e))
}

/// Parse a 256-bit hash from either hex string or Solidity event definition
///
/// Supports two formats:
/// 1. Hex string: `0x1234...`
/// 2. Event definition: `event Deposit(address indexed from, uint256 amount)`
///
/// # Examples
/// ```
/// use synddb_shared::parse::parse_b256;
///
/// // Parse from hex
/// let hash = parse_b256("0x0000000000000000000000000000000000000000000000000000000000000001").unwrap();
///
/// // Parse from event definition
/// let event_hash = parse_b256("event Deposit(address indexed from, uint256 amount)").unwrap();
/// ```
pub fn parse_b256(s: &str) -> Result<B256, String> {
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
/// - `event Deposit(address indexed from, uint256 amount)`
/// - `Deposit(address,uint256)` (canonical signature)
///
/// Returns the keccak256 hash of the canonical event signature.
///
/// # Examples
/// ```
/// use synddb_shared::parse::parse_event_signature_from_definition;
///
/// let hash = parse_event_signature_from_definition("event Deposit(address indexed from, uint256 amount)").unwrap();
/// ```
pub fn parse_event_signature_from_definition(s: &str) -> Result<B256, String> {
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
/// Example: `event Deposit(address indexed from, uint256 amount)`
/// Returns: `Deposit(address,uint256)`
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_canonical_signature() {
        let event_def = "event Deposit(address indexed from, uint256 amount)";
        let canonical = extract_canonical_signature(event_def);
        assert_eq!(canonical, Some("Deposit(address,uint256)".to_string()));
    }

    #[test]
    fn test_extract_canonical_signature_no_indexed() {
        let event_def = "event Transfer(address from, address to, uint256 value)";
        let canonical = extract_canonical_signature(event_def);
        assert_eq!(
            canonical,
            Some("Transfer(address,address,uint256)".to_string())
        );
    }

    #[test]
    fn test_parse_event_signature() {
        // Should compute keccak256 hash
        let result = parse_event_signature_from_definition(
            "event Deposit(address indexed from, uint256 amount)",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_b256_hex() {
        let hash = parse_b256("0x0000000000000000000000000000000000000000000000000000000000000001");
        assert!(hash.is_ok());
    }

    #[test]
    fn test_parse_address_valid() {
        let addr = parse_address("0x742d35Cc6634C0532925a3b844Bc454e4438f44e");
        assert!(addr.is_ok());
    }

    #[test]
    fn test_parse_address_invalid() {
        let addr = parse_address("invalid");
        assert!(addr.is_err());
    }

    #[test]
    fn test_parse_url_valid() {
        let url = parse_url("https://example.com");
        assert!(url.is_ok());
    }

    #[test]
    fn test_parse_url_invalid() {
        let url = parse_url("not a url");
        assert!(url.is_err());
    }
}
