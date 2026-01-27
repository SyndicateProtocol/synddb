//! Price consistency validation rule
//!
//! This rule ensures that prices from multiple sources agree within a configurable
//! tolerance. It queries the database after changeset application to check that
//! CoinGecko and CoinMarketCap prices for the same asset don't differ by more
//! than the allowed threshold.
//!
//! Key insight: The validator doesn't need API keys! It only queries the database
//! where the application has already logged prices from both sources.

use anyhow::Result;
use rusqlite::Connection;
use synddb_validator::rules::{ValidationResult, ValidationRule};
use tracing::{debug, info, warn};

/// Validates that prices from different sources are consistent
///
/// The rule checks the `prices` table for the most recent prices from each source
/// and ensures they don't differ by more than the configured tolerance.
pub struct PriceConsistencyRule {
    /// Maximum allowed difference in basis points (100 bps = 1%)
    max_difference_bps: u32,
    /// Whether the rule is enabled
    enabled: bool,
}

impl PriceConsistencyRule {
    /// Create a new price consistency rule with the given tolerance
    ///
    /// # Arguments
    ///
    /// * `max_difference_bps` - Maximum allowed price difference in basis points
    ///   (e.g., 100 = 1%, 50 = 0.5%)
    pub fn new(max_difference_bps: u32) -> Self {
        Self {
            max_difference_bps,
            enabled: true,
        }
    }

    /// Set whether this rule is enabled
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Calculate the difference between two prices in basis points
    fn calculate_difference_bps(price1: f64, price2: f64) -> u32 {
        if price1 == 0.0 || price2 == 0.0 {
            return u32::MAX;
        }
        let avg = (price1 + price2) / 2.0;
        let diff = (price1 - price2).abs();
        ((diff / avg) * 10000.0) as u32
    }
}

impl ValidationRule for PriceConsistencyRule {
    fn name(&self) -> &str {
        "price_consistency"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn validate(&self, conn: &Connection, sequence: u64) -> Result<ValidationResult> {
        // Query for the latest prices from each source for each asset
        // The prices table schema:
        //   CREATE TABLE prices (
        //       id INTEGER PRIMARY KEY,
        //       asset TEXT NOT NULL,
        //       source TEXT NOT NULL,  -- 'coingecko' or 'coinmarketcap'
        //       price REAL NOT NULL,
        //       timestamp INTEGER NOT NULL
        //   );

        // Check if prices table exists
        let table_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='prices')",
            [],
            |row| row.get(0),
        )?;

        if !table_exists {
            debug!(sequence, "prices table does not exist, rule not applicable");
            return Ok(ValidationResult::NotApplicable);
        }

        // Get all assets that have prices from both sources
        let mut stmt = conn.prepare(
            r#"
            SELECT DISTINCT p1.asset
            FROM prices p1
            WHERE EXISTS (
                SELECT 1 FROM prices p2
                WHERE p2.asset = p1.asset
                AND p2.source != p1.source
            )
            "#,
        )?;

        let assets: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;

        if assets.is_empty() {
            debug!(
                sequence,
                "No assets with multiple sources, rule not applicable"
            );
            return Ok(ValidationResult::NotApplicable);
        }

        // Check each asset for price consistency
        for asset in &assets {
            // Get latest prices from each source
            let mut price_stmt = conn.prepare(
                r#"
                SELECT source, price
                FROM prices
                WHERE asset = ?1
                ORDER BY timestamp DESC
                "#,
            )?;

            let prices: Vec<(String, f64)> = price_stmt
                .query_map([asset], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;

            // Group by source, taking most recent
            let mut coingecko_price: Option<f64> = None;
            let mut coinmarketcap_price: Option<f64> = None;

            for (source, price) in prices {
                match source.as_str() {
                    "coingecko" if coingecko_price.is_none() => {
                        coingecko_price = Some(price);
                    }
                    "coinmarketcap" if coinmarketcap_price.is_none() => {
                        coinmarketcap_price = Some(price);
                    }
                    _ => {}
                }
                // Early exit if we have both
                if coingecko_price.is_some() && coinmarketcap_price.is_some() {
                    break;
                }
            }

            // Check consistency if we have both prices
            if let (Some(cg_price), Some(cmc_price)) = (coingecko_price, coinmarketcap_price) {
                let diff_bps = Self::calculate_difference_bps(cg_price, cmc_price);

                info!(
                    sequence,
                    asset = %asset,
                    coingecko_price = cg_price,
                    coinmarketcap_price = cmc_price,
                    difference_bps = diff_bps,
                    max_allowed_bps = self.max_difference_bps,
                    "Checking price consistency"
                );

                if diff_bps > self.max_difference_bps {
                    let reason = format!(
                        "Price difference for {} exceeds threshold: CoinGecko=${:.4}, CoinMarketCap=${:.4}, \
                         difference={}bps (max allowed={}bps)",
                        asset, cg_price, cmc_price, diff_bps, self.max_difference_bps
                    );
                    warn!(
                        sequence,
                        asset = %asset,
                        coingecko_price = cg_price,
                        coinmarketcap_price = cmc_price,
                        difference_bps = diff_bps,
                        max_allowed_bps = self.max_difference_bps,
                        "Price consistency check FAILED"
                    );
                    return Ok(ValidationResult::Fail { reason });
                }

                debug!(
                    sequence,
                    asset = %asset,
                    difference_bps = diff_bps,
                    "Price consistency check passed"
                );
            }
        }

        Ok(ValidationResult::Pass)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE prices (
                id INTEGER PRIMARY KEY,
                asset TEXT NOT NULL,
                source TEXT NOT NULL,
                price REAL NOT NULL,
                timestamp INTEGER NOT NULL
            );
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_calculate_difference_bps() {
        // 1% difference
        assert_eq!(
            PriceConsistencyRule::calculate_difference_bps(100.0, 101.0),
            99 // ~1%
        );

        // 0% difference
        assert_eq!(
            PriceConsistencyRule::calculate_difference_bps(100.0, 100.0),
            0
        );

        // 10% difference
        assert_eq!(
            PriceConsistencyRule::calculate_difference_bps(100.0, 110.0),
            952 // ~9.5%
        );

        // Zero price handling
        assert_eq!(
            PriceConsistencyRule::calculate_difference_bps(0.0, 100.0),
            u32::MAX
        );
    }

    #[test]
    fn test_no_prices_table() {
        let conn = Connection::open_in_memory().unwrap();
        let rule = PriceConsistencyRule::new(100);

        let result = rule.validate(&conn, 1).unwrap();
        assert_eq!(result, ValidationResult::NotApplicable);
    }

    #[test]
    fn test_empty_prices() {
        let conn = setup_test_db();
        let rule = PriceConsistencyRule::new(100);

        let result = rule.validate(&conn, 1).unwrap();
        assert_eq!(result, ValidationResult::NotApplicable);
    }

    #[test]
    fn test_single_source_only() {
        let conn = setup_test_db();
        conn.execute(
            "INSERT INTO prices (asset, source, price, timestamp) VALUES ('BTC', 'coingecko', 50000.0, 1000)",
            [],
        )
        .unwrap();

        let rule = PriceConsistencyRule::new(100);
        let result = rule.validate(&conn, 1).unwrap();
        assert_eq!(result, ValidationResult::NotApplicable);
    }

    #[test]
    fn test_prices_within_tolerance() {
        let conn = setup_test_db();

        // BTC prices within 1%
        conn.execute(
            "INSERT INTO prices (asset, source, price, timestamp) VALUES ('BTC', 'coingecko', 50000.0, 1000)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO prices (asset, source, price, timestamp) VALUES ('BTC', 'coinmarketcap', 50250.0, 1001)",
            [],
        ).unwrap();

        let rule = PriceConsistencyRule::new(100); // 1% tolerance
        let result = rule.validate(&conn, 1).unwrap();
        assert_eq!(result, ValidationResult::Pass);
    }

    #[test]
    fn test_prices_exceed_tolerance() {
        let conn = setup_test_db();

        // BTC prices differ by more than 1%
        conn.execute(
            "INSERT INTO prices (asset, source, price, timestamp) VALUES ('BTC', 'coingecko', 50000.0, 1000)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO prices (asset, source, price, timestamp) VALUES ('BTC', 'coinmarketcap', 51000.0, 1001)",
            [],
        ).unwrap();

        let rule = PriceConsistencyRule::new(100); // 1% tolerance
        let result = rule.validate(&conn, 1).unwrap();

        match result {
            ValidationResult::Fail { reason } => {
                assert!(reason.contains("BTC"));
                assert!(reason.contains("exceeds threshold"));
            }
            _ => panic!("Expected Fail result"),
        }
    }

    #[test]
    fn test_multiple_assets() {
        let conn = setup_test_db();

        // BTC within tolerance
        conn.execute(
            "INSERT INTO prices (asset, source, price, timestamp) VALUES ('BTC', 'coingecko', 50000.0, 1000)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO prices (asset, source, price, timestamp) VALUES ('BTC', 'coinmarketcap', 50100.0, 1001)",
            [],
        ).unwrap();

        // ETH exceeds tolerance
        conn.execute(
            "INSERT INTO prices (asset, source, price, timestamp) VALUES ('ETH', 'coingecko', 3000.0, 1000)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO prices (asset, source, price, timestamp) VALUES ('ETH', 'coinmarketcap', 3100.0, 1001)",
            [],
        ).unwrap();

        let rule = PriceConsistencyRule::new(100); // 1% tolerance
        let result = rule.validate(&conn, 1).unwrap();

        // Should fail because ETH exceeds threshold
        match result {
            ValidationResult::Fail { reason } => {
                assert!(reason.contains("ETH"));
            }
            _ => panic!("Expected Fail result for ETH"),
        }
    }

    #[test]
    fn test_disabled_rule() {
        let mut rule = PriceConsistencyRule::new(100);
        assert!(rule.is_enabled());

        rule.set_enabled(false);
        assert!(!rule.is_enabled());
    }

    #[test]
    fn test_uses_latest_prices() {
        let conn = setup_test_db();

        // Old prices that differ significantly
        conn.execute(
            "INSERT INTO prices (asset, source, price, timestamp) VALUES ('BTC', 'coingecko', 40000.0, 1000)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO prices (asset, source, price, timestamp) VALUES ('BTC', 'coinmarketcap', 50000.0, 1001)",
            [],
        ).unwrap();

        // New prices that are consistent
        conn.execute(
            "INSERT INTO prices (asset, source, price, timestamp) VALUES ('BTC', 'coingecko', 50000.0, 2000)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO prices (asset, source, price, timestamp) VALUES ('BTC', 'coinmarketcap', 50100.0, 2001)",
            [],
        ).unwrap();

        let rule = PriceConsistencyRule::new(100);
        let result = rule.validate(&conn, 1).unwrap();

        // Should pass using latest prices
        assert_eq!(result, ValidationResult::Pass);
    }
}
