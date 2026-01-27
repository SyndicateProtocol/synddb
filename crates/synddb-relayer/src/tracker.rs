//! Funding rate limiter and tracker
//!
//! Tracks funding requests per-application to enforce per-digest and per-address caps.

use crate::config::ApplicationConfig;
use alloy::primitives::{Address, B256};
use std::{
    collections::HashMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Key for per-application, per-digest tracking
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DigestKey {
    audience_hash: B256,
    image_digest: B256,
}

/// Key for per-application, per-address tracking
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AddressKey {
    audience_hash: B256,
    address: Address,
}

/// Tracks funding to enforce per-application rate limits
#[derive(Debug, Default)]
pub(crate) struct FundingTracker {
    /// Per (application, `image_digest`) tracking: (`total_funded_today`, `last_reset_timestamp`)
    per_digest: HashMap<DigestKey, (u128, u64)>,

    /// Per (application, address) tracking: `total_funded`
    per_address: HashMap<AddressKey, u128>,
}

impl FundingTracker {
    /// Create a new tracker
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Check if funding is allowed for a given application, digest, and address
    ///
    /// Uses the application-specific caps from the config.
    /// Returns Ok(()) if allowed, Err with reason if denied.
    pub(crate) fn check_allowed(
        &mut self,
        audience_hash: &B256,
        image_digest: &B256,
        address: Address,
        funding_amount: u128,
        app_config: &ApplicationConfig,
    ) -> Result<(), String> {
        let now = current_timestamp();
        let day_start = now - (now % 86400); // Start of current day (UTC)

        // Check per-digest limit for this application
        let digest_key = DigestKey {
            audience_hash: *audience_hash,
            image_digest: *image_digest,
        };
        let (funded_today, last_reset) =
            self.per_digest.entry(digest_key).or_insert((0, day_start));

        // Reset daily counter if new day
        if *last_reset < day_start {
            *funded_today = 0;
            *last_reset = day_start;
        }

        let new_digest_total = *funded_today + funding_amount;
        if new_digest_total > app_config.max_funding_per_digest_daily {
            return Err(format!(
                "Per-digest daily limit exceeded: {} funded, {} max",
                *funded_today, app_config.max_funding_per_digest_daily
            ));
        }

        // Check per-address limit for this application
        let address_key = AddressKey {
            audience_hash: *audience_hash,
            address,
        };
        let address_total = self.per_address.entry(address_key).or_insert(0);
        let new_address_total = *address_total + funding_amount;
        if new_address_total > app_config.max_funding_per_address {
            return Err(format!(
                "Per-address limit exceeded: {} funded, {} max",
                *address_total, app_config.max_funding_per_address
            ));
        }

        Ok(())
    }

    /// Record a successful funding
    pub(crate) fn record_funding(
        &mut self,
        audience_hash: &B256,
        image_digest: &B256,
        address: Address,
        amount: u128,
    ) {
        let now = current_timestamp();
        let day_start = now - (now % 86400);

        // Update per-digest tracking
        let digest_key = DigestKey {
            audience_hash: *audience_hash,
            image_digest: *image_digest,
        };
        let (funded_today, last_reset) =
            self.per_digest.entry(digest_key).or_insert((0, day_start));

        if *last_reset < day_start {
            *funded_today = amount;
            *last_reset = day_start;
        } else {
            *funded_today += amount;
        }

        // Update per-address tracking
        let address_key = AddressKey {
            audience_hash: *audience_hash,
            address,
        };
        *self.per_address.entry(address_key).or_insert(0) += amount;
    }

    /// Get current funding stats for an address in an application
    pub(crate) fn get_address_funded(&self, audience_hash: &B256, address: &Address) -> u128 {
        let key = AddressKey {
            audience_hash: *audience_hash,
            address: *address,
        };
        self.per_address.get(&key).copied().unwrap_or(0)
    }

    /// Get current daily funding for a digest in an application
    pub(crate) fn get_digest_funded_today(
        &self,
        audience_hash: &B256,
        image_digest: &B256,
    ) -> u128 {
        let now = current_timestamp();
        let day_start = now - (now % 86400);

        let key = DigestKey {
            audience_hash: *audience_hash,
            image_digest: *image_digest,
        };

        self.per_digest
            .get(&key)
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

    fn test_app_config() -> ApplicationConfig {
        ApplicationConfig {
            audience_hash: B256::ZERO,
            treasury_address: Address::ZERO,
            allowed_image_digests: vec![B256::ZERO],
            max_funding_per_digest_daily: 1_000_000_000_000_000_000, // 1 ETH
            max_funding_per_address: 50_000_000_000_000_000,         // 0.05 ETH
        }
    }

    #[test]
    fn test_tracker_allows_first_request() {
        let mut tracker = FundingTracker::new();
        let app_config = test_app_config();
        let audience = B256::ZERO;
        let digest = B256::from([0x11; 32]);
        let address = Address::ZERO;
        let amount = 10_000_000_000_000_000u128; // 0.01 ETH

        let result = tracker.check_allowed(&audience, &digest, address, amount, &app_config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_tracker_denies_over_address_limit() {
        let mut tracker = FundingTracker::new();
        let app_config = test_app_config();
        let audience = B256::ZERO;
        let digest = B256::from([0x11; 32]);
        let address = Address::ZERO;

        // Record max allowed
        tracker.record_funding(
            &audience,
            &digest,
            address,
            app_config.max_funding_per_address,
        );

        // Next request should be denied
        let result = tracker.check_allowed(&audience, &digest, address, 1, &app_config);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Per-address limit"));
    }

    #[test]
    fn test_tracker_denies_over_digest_limit() {
        let mut tracker = FundingTracker::new();
        let app_config = test_app_config();
        let audience = B256::ZERO;
        let digest = B256::from([0x11; 32]);

        // Record max allowed across multiple addresses
        for i in 0..20 {
            let addr = Address::from_slice(&[i as u8; 20]);
            tracker.record_funding(
                &audience,
                &digest,
                addr,
                app_config.max_funding_per_digest_daily / 20,
            );
        }

        // Next request from new address should be denied (digest limit)
        let new_addr = Address::from_slice(&[0xFF; 20]);
        let result = tracker.check_allowed(&audience, &digest, new_addr, 1, &app_config);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Per-digest"));
    }

    #[test]
    fn test_tracker_isolates_applications() {
        let mut tracker = FundingTracker::new();
        let app_config = test_app_config();
        let audience1 = B256::from([0x11; 32]);
        let audience2 = B256::from([0x22; 32]);
        let digest = B256::from([0x33; 32]);
        let address = Address::ZERO;

        // Max out funding for app1
        tracker.record_funding(
            &audience1,
            &digest,
            address,
            app_config.max_funding_per_address,
        );

        // App2 should still allow funding (separate tracking)
        let result = tracker.check_allowed(
            &audience2,
            &digest,
            address,
            app_config.max_funding_per_address,
            &app_config,
        );
        assert!(result.is_ok());
    }
}
