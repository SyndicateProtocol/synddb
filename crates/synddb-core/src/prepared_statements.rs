//! Prepared statement cache for high-performance SQL execution
//!
//! This module provides a cache for prepared SQL statements to avoid
//! re-parsing and re-compiling frequently used queries.

use parking_lot::RwLock;
use std::collections::HashMap;
use tracing::debug;

/// Cache for prepared SQL statements
///
/// Stores SQL strings keyed by identifier for fast lookup.
/// Thread-safe using parking_lot's RwLock for low-latency concurrent access.
pub struct PreparedStatementCache {
    /// Map of statement key to SQL string
    statements: RwLock<HashMap<String, String>>,
    /// Cache hit/miss statistics
    stats: RwLock<CacheStats>,
}

/// Statistics for cache performance
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Total number of cache lookups
    pub lookups: u64,
    /// Number of cache hits
    pub hits: u64,
    /// Number of cache misses
    pub misses: u64,
}

impl CacheStats {
    /// Calculate hit rate as a percentage
    pub fn hit_rate(&self) -> f64 {
        if self.lookups == 0 {
            return 0.0;
        }
        (self.hits as f64 / self.lookups as f64) * 100.0
    }
}

impl PreparedStatementCache {
    /// Create a new empty cache
    pub fn new() -> Self {
        Self {
            statements: RwLock::new(HashMap::new()),
            stats: RwLock::new(CacheStats::default()),
        }
    }

    /// Register a prepared statement in the cache
    ///
    /// # Arguments
    /// * `key` - Unique identifier for this statement
    /// * `sql` - SQL statement to cache
    pub fn register(&self, key: impl Into<String>, sql: impl Into<String>) {
        let key = key.into();
        let sql = sql.into();

        let mut cache = self.statements.write();
        if cache.insert(key.clone(), sql).is_none() {
            debug!("Registered prepared statement: {}", key);
        }
    }

    /// Get a prepared statement from the cache
    ///
    /// # Arguments
    /// * `key` - Identifier for the statement
    ///
    /// # Returns
    /// The SQL string if found, None otherwise
    pub fn get(&self, key: &str) -> Option<String> {
        let cache = self.statements.read();
        let result = cache.get(key).cloned();

        // Update statistics
        let mut stats = self.stats.write();
        stats.lookups += 1;
        if result.is_some() {
            stats.hits += 1;
        } else {
            stats.misses += 1;
        }

        result
    }

    /// Remove a statement from the cache
    pub fn remove(&self, key: &str) -> Option<String> {
        let mut cache = self.statements.write();
        cache.remove(key)
    }

    /// Clear all cached statements
    pub fn clear(&self) {
        let mut cache = self.statements.write();
        cache.clear();
        debug!("Cleared prepared statement cache");
    }

    /// Get the number of cached statements
    pub fn len(&self) -> usize {
        let cache = self.statements.read();
        cache.len()
    }

    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        let cache = self.statements.read();
        cache.is_empty()
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        let stats = self.stats.read();
        stats.clone()
    }

    /// Reset statistics
    pub fn reset_stats(&self) {
        let mut stats = self.stats.write();
        *stats = CacheStats::default();
    }

    /// Initialize common prepared statements
    ///
    /// This pre-registers frequently used SQL patterns that extensions
    /// commonly need. Extensions can add their own via `register()`.
    pub fn initialize_common(&self) {
        // Order book operations
        self.register(
            "insert_order",
            r#"
            INSERT INTO orders (order_id, account_id, side, price, quantity,
                              remaining_quantity, status, created_at, updated_at, nonce)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        );

        self.register(
            "update_order_status",
            r#"
            UPDATE orders
            SET status = ?1, remaining_quantity = ?2, updated_at = ?3
            WHERE order_id = ?4
            "#,
        );

        self.register(
            "cancel_order",
            r#"
            UPDATE orders
            SET status = 'CANCELED', updated_at = ?1
            WHERE order_id = ?2 AND account_id = ?3 AND status = 'OPEN'
            "#,
        );

        self.register(
            "get_order_by_id",
            r#"
            SELECT * FROM orders WHERE order_id = ?1
            "#,
        );

        self.register(
            "get_open_orders",
            r#"
            SELECT * FROM orders
            WHERE account_id = ?1 AND status = 'OPEN'
            ORDER BY created_at DESC
            "#,
        );

        // Balance operations
        self.register(
            "get_balance",
            r#"
            SELECT balance FROM balances
            WHERE account_id = ?1 AND token_address = ?2
            "#,
        );

        self.register(
            "update_balance",
            r#"
            UPDATE balances
            SET balance = balance + ?1, updated_at = ?2
            WHERE account_id = ?3 AND token_address = ?4
            "#,
        );

        self.register(
            "insert_balance",
            r#"
            INSERT INTO balances (account_id, token_address, balance, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT (account_id, token_address)
            DO UPDATE SET balance = balance + excluded.balance, updated_at = excluded.updated_at
            "#,
        );

        // Transfer operations
        self.register(
            "insert_transfer",
            r#"
            INSERT INTO transfers (transfer_id, from_account, to_account, token_address,
                                 amount, timestamp, nonce)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        );

        self.register(
            "get_transfers_by_account",
            r#"
            SELECT * FROM transfers
            WHERE from_account = ?1 OR to_account = ?1
            ORDER BY timestamp DESC
            LIMIT ?2
            "#,
        );

        // Trade operations
        self.register(
            "insert_trade",
            r#"
            INSERT INTO trades (trade_id, buy_order_id, sell_order_id, price,
                              quantity, timestamp)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        );

        self.register(
            "get_recent_trades",
            r#"
            SELECT * FROM trades
            WHERE timestamp >= ?1
            ORDER BY timestamp DESC
            LIMIT ?2
            "#,
        );

        // Withdrawal/deposit operations
        self.register(
            "insert_withdrawal_request",
            r#"
            INSERT INTO withdrawal_requests (request_id, account_id, token_address,
                                            amount, destination_address, status,
                                            timestamp)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        );

        self.register(
            "update_withdrawal_status",
            r#"
            UPDATE withdrawal_requests
            SET status = ?1, settlement_tx_hash = ?2, updated_at = ?3
            WHERE request_id = ?4
            "#,
        );

        self.register(
            "get_pending_withdrawals",
            r#"
            SELECT * FROM withdrawal_requests
            WHERE status = 'PENDING'
            ORDER BY timestamp ASC
            LIMIT ?1
            "#,
        );

        self.register(
            "insert_deposit",
            r#"
            INSERT INTO deposits (deposit_id, account_id, token_address, amount,
                                source_tx_hash, timestamp)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        );

        debug!("Initialized {} common prepared statements", self.len());
    }
}

impl Default for PreparedStatementCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_basic_operations() {
        let cache = PreparedStatementCache::new();

        // Register a statement
        cache.register("test_query", "SELECT * FROM test WHERE id = ?1");

        // Retrieve it
        let sql = cache.get("test_query");
        assert!(sql.is_some());
        assert_eq!(sql.unwrap(), "SELECT * FROM test WHERE id = ?1");

        // Check stats
        let stats = cache.stats();
        assert_eq!(stats.lookups, 1);
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 0);

        // Try to get non-existent statement
        let missing = cache.get("nonexistent");
        assert!(missing.is_none());

        let stats = cache.stats();
        assert_eq!(stats.lookups, 2);
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_cache_hit_rate() {
        let cache = PreparedStatementCache::new();
        cache.register("query1", "SELECT 1");

        // 3 hits
        cache.get("query1");
        cache.get("query1");
        cache.get("query1");

        // 2 misses
        cache.get("query2");
        cache.get("query3");

        let stats = cache.stats();
        assert_eq!(stats.lookups, 5);
        assert_eq!(stats.hits, 3);
        assert_eq!(stats.misses, 2);
        assert_eq!(stats.hit_rate(), 60.0);
    }

    #[test]
    fn test_cache_clear() {
        let cache = PreparedStatementCache::new();
        cache.register("query1", "SELECT 1");
        cache.register("query2", "SELECT 2");

        assert_eq!(cache.len(), 2);

        cache.clear();
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cache_remove() {
        let cache = PreparedStatementCache::new();
        cache.register("query1", "SELECT 1");

        let removed = cache.remove("query1");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap(), "SELECT 1");

        assert!(cache.is_empty());
    }

    #[test]
    fn test_initialize_common() {
        let cache = PreparedStatementCache::new();
        cache.initialize_common();

        // Should have multiple common statements
        assert!(cache.len() > 0);

        // Test a few specific ones
        assert!(cache.get("insert_order").is_some());
        assert!(cache.get("get_balance").is_some());
        assert!(cache.get("insert_trade").is_some());
    }

    #[test]
    fn test_stats_reset() {
        let cache = PreparedStatementCache::new();
        cache.register("query1", "SELECT 1");

        cache.get("query1");
        cache.get("query1");

        let stats = cache.stats();
        assert_eq!(stats.lookups, 2);

        cache.reset_stats();

        let stats = cache.stats();
        assert_eq!(stats.lookups, 0);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
    }
}
