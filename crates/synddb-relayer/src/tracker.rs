//! Funding rate limiter and tracker
//!
//! Tracks funding requests to enforce per-digest and per-address caps.

use crate::config::RelayerConfig;
use alloy::primitives::{Address, B256};
use std::{
    collections::HashMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Tracks funding to enforce rate limits
#[derive(Debug)]
pub struct FundingTracker {
    /// Per image digest tracking: (total_funded_today, last_reset_timestamp)
    per_digest: HashMap<B256, (u128, u64)>,

    /// Per address tracking: total_funded
    per_address: HashMap<Address, u128>,

    /// Maximum funding per digest per day
    max_per_digest_daily: u128,

    /// Maximum funding per address
    max_per_address: u128,
}

impl FundingTracker {
    /// Create a new tracker from config
    pub fn new(config: &RelayerConfig) -> Self {
        Self {
            per_digest: HashMap::new(),
            per_address: HashMap::new(),
            max_per_digest_daily: config.max_funding_per_digest_daily,
            max_per_address: config.max_funding_per_address,
        }
    }

    /// Check if funding is allowed for a given digest and address
    ///
    /// Returns Ok(()) if allowed, Err with reason if denied.
    pub fn check_allowed(
        &mut self,
        image_digest: B256,
        address: Address,
        funding_amount: u128,
    ) -> Result<(), String> {
        let now = current_timestamp();
        let day_start = now - (now % 86400); // Start of current day (UTC)

        // Check per-digest limit
        let (funded_today, last_reset) = self
            .per_digest
            .entry(image_digest)
            .or_insert((0, day_start));

        // Reset daily counter if new day
        if *last_reset < day_start {
            *funded_today = 0;
            *last_reset = day_start;
        }

        let new_digest_total = *funded_today + funding_amount;
        if new_digest_total > self.max_per_digest_daily {
            return Err(format!(
                "Per-digest daily limit exceeded: {} funded, {} max",
                *funded_today, self.max_per_digest_daily
            ));
        }

        // Check per-address limit
        let address_total = self.per_address.entry(address).or_insert(0);
        let new_address_total = *address_total + funding_amount;
        if new_address_total > self.max_per_address {
            return Err(format!(
                "Per-address limit exceeded: {} funded, {} max",
                *address_total, self.max_per_address
            ));
        }

        Ok(())
    }

    /// Record a successful funding
    pub fn record_funding(&mut self, image_digest: B256, address: Address, amount: u128) {
        let now = current_timestamp();
        let day_start = now - (now % 86400);

        // Update per-digest tracking
        let (funded_today, last_reset) = self
            .per_digest
            .entry(image_digest)
            .or_insert((0, day_start));

        if *last_reset < day_start {
            *funded_today = amount;
            *last_reset = day_start;
        } else {
            *funded_today += amount;
        }

        // Update per-address tracking
        *self.per_address.entry(address).or_insert(0) += amount;
    }

    /// Get current funding stats for an address
    pub fn get_address_funded(&self, address: &Address) -> u128 {
        *self.per_address.get(address).unwrap_or(&0)
    }

    /// Get current daily funding for a digest
    pub fn get_digest_funded_today(&self, digest: &B256) -> u128 {
        let now = current_timestamp();
        let day_start = now - (now % 86400);

        self.per_digest
            .get(digest)
            .filter(|(_, last_reset)| *last_reset >= day_start)
            .map_or(0, |(funded, _)| *funded)
    }
}

/// Get current Unix timestamp
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> RelayerConfig {
        RelayerConfig {
            rpc_url: "http://localhost:8545".to_string(),
            chain_id: 1,
            key_manager_address: Address::ZERO,
            treasury_address: Address::ZERO,
            private_key: "0x".to_string() + &"ab".repeat(32),
            listen_addr: "0.0.0.0:8082".parse().unwrap(),
            allowed_image_digests: "0x".to_string() + &"cd".repeat(32),
            max_funding_per_digest_daily: 1_000_000_000_000_000_000, // 1 ETH
            max_funding_per_address: 50_000_000_000_000_000,         // 0.05 ETH
        }
    }

    #[test]
    fn test_tracker_allows_first_request() {
        let mut tracker = FundingTracker::new(&test_config());
        let digest = B256::ZERO;
        let address = Address::ZERO;
        let amount = 10_000_000_000_000_000u128; // 0.01 ETH

        let result = tracker.check_allowed(digest, address, amount);
        assert!(result.is_ok());
    }

    #[test]
    fn test_tracker_denies_over_address_limit() {
        let config = test_config();
        let mut tracker = FundingTracker::new(&config);
        let digest = B256::ZERO;
        let address = Address::ZERO;

        // Record max allowed
        tracker.record_funding(digest, address, config.max_funding_per_address);

        // Next request should be denied
        let result = tracker.check_allowed(digest, address, 1);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Per-address limit"));
    }

    #[test]
    fn test_tracker_denies_over_digest_limit() {
        let config = test_config();
        let mut tracker = FundingTracker::new(&config);
        let digest = B256::ZERO;

        // Record max allowed across multiple addresses
        for i in 0..20 {
            let addr = Address::from_slice(&[i as u8; 20]);
            tracker.record_funding(digest, addr, config.max_funding_per_digest_daily / 20);
        }

        // Next request from new address should be denied (digest limit)
        let new_addr = Address::from_slice(&[0xFF; 20]);
        let result = tracker.check_allowed(digest, new_addr, 1);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Per-digest"));
    }
}
