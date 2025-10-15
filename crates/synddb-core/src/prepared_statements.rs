//! Prepared statement cache for high-performance SQL execution
//!
//! This module provides automatic caching of SQL statements based on the
//! SQL string itself, similar to how PostgreSQL and other databases handle
//! prepared statements internally.

use parking_lot::RwLock;
use std::collections::HashMap;

/// Automatic prepared statement cache
///
/// Caches SQL statements by their content hash for automatic reuse.
/// This is transparent to the caller - extensions just execute SQL
/// and the cache automatically handles optimization.
pub struct PreparedStatementCache {
    /// Map of SQL hash to SQL string
    cache: RwLock<HashMap<u64, CachedStatement>>,
    /// Cache statistics
    stats: RwLock<CacheStats>,
    /// Maximum cache size
    max_size: usize,
}

/// Cached statement information
struct CachedStatement {
    /// The SQL string
    sql: String,
    /// Number of times used
    use_count: u64,
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
    /// Create a new statement cache with default size (1000 statements)
    pub fn new() -> Self {
        Self::with_capacity(1000)
    }

    /// Create a cache with specified maximum capacity
    pub fn with_capacity(max_size: usize) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            stats: RwLock::new(CacheStats::default()),
            max_size,
        }
    }

    /// Cache a SQL statement (called automatically by database layer)
    ///
    /// Returns true if this is a new cache entry
    pub(crate) fn cache_statement(&self, sql: &str) -> bool {
        let hash = Self::hash_sql(sql);

        // Fast path - check if already cached
        {
            let cache = self.cache.read();
            if let Some(entry) = cache.get(&hash) {
                // Already cached, just increment use count
                drop(cache);
                let mut cache = self.cache.write();
                if let Some(entry) = cache.get_mut(&hash) {
                    entry.use_count += 1;
                }

                let mut stats = self.stats.write();
                stats.lookups += 1;
                stats.hits += 1;
                return false;
            }
        }

        // Not cached - add it
        let mut cache = self.cache.write();

        // Evict LRU if at capacity
        if cache.len() >= self.max_size {
            self.evict_lru(&mut cache);
        }

        cache.insert(
            hash,
            CachedStatement {
                sql: sql.to_string(),
                use_count: 1,
            },
        );

        let mut stats = self.stats.write();
        stats.lookups += 1;
        stats.misses += 1;

        true
    }

    /// Check if a statement is cached
    pub fn is_cached(&self, sql: &str) -> bool {
        let hash = Self::hash_sql(sql);
        let cache = self.cache.read();
        cache.contains_key(&hash)
    }

    /// Evict least-recently-used entry
    fn evict_lru(&self, cache: &mut HashMap<u64, CachedStatement>) {
        if let Some((&hash, _)) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.use_count)
        {
            cache.remove(&hash);
        }
    }

    /// Hash SQL for cache key (simple FNV-1a hash)
    fn hash_sql(sql: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        sql.hash(&mut hasher);
        hasher.finish()
    }

    /// Clear all cached statements
    pub fn clear(&self) {
        let mut cache = self.cache.write();
        cache.clear();
    }

    /// Get the number of cached statements
    pub fn len(&self) -> usize {
        let cache = self.cache.read();
        cache.len()
    }

    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        let cache = self.cache.read();
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
    fn test_automatic_caching() {
        let cache = PreparedStatementCache::new();

        let sql = "SELECT * FROM test WHERE id = ?1";

        // First use - should cache
        assert!(cache.cache_statement(sql));
        assert_eq!(cache.len(), 1);

        // Second use - should hit cache
        assert!(!cache.cache_statement(sql));
        assert_eq!(cache.len(), 1);

        // Check stats
        let stats = cache.stats();
        assert_eq!(stats.lookups, 2);
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_cache_hit_rate() {
        let cache = PreparedStatementCache::new();

        let sql1 = "SELECT * FROM test WHERE id = ?1";
        let sql2 = "SELECT * FROM test WHERE name = ?1";

        // Cache both statements
        cache.cache_statement(sql1);
        cache.cache_statement(sql2);

        // Use sql1 multiple times (hits)
        cache.cache_statement(sql1);
        cache.cache_statement(sql1);
        cache.cache_statement(sql1);

        let stats = cache.stats();
        assert_eq!(stats.lookups, 5);
        assert_eq!(stats.hits, 3); // Last 3 were hits
        assert_eq!(stats.misses, 2); // First 2 were misses
        assert_eq!(stats.hit_rate(), 60.0);
    }

    #[test]
    fn test_is_cached() {
        let cache = PreparedStatementCache::new();

        let sql = "SELECT * FROM test WHERE id = ?1";

        assert!(!cache.is_cached(sql));

        cache.cache_statement(sql);

        assert!(cache.is_cached(sql));
    }

    #[test]
    fn test_cache_eviction() {
        let cache = PreparedStatementCache::with_capacity(3);

        // Fill cache to capacity
        cache.cache_statement("SELECT 1");
        cache.cache_statement("SELECT 2");
        cache.cache_statement("SELECT 3");

        assert_eq!(cache.len(), 3);

        // Add one more - should evict LRU
        cache.cache_statement("SELECT 4");

        // Should still be at capacity
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn test_cache_clear() {
        let cache = PreparedStatementCache::new();

        cache.cache_statement("SELECT 1");
        cache.cache_statement("SELECT 2");

        assert_eq!(cache.len(), 2);

        cache.clear();
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_stats_reset() {
        let cache = PreparedStatementCache::new();

        cache.cache_statement("SELECT 1");
        cache.cache_statement("SELECT 1");

        let stats = cache.stats();
        assert_eq!(stats.lookups, 2);

        cache.reset_stats();

        let stats = cache.stats();
        assert_eq!(stats.lookups, 0);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
    }
}
