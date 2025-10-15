//! Extension system for SyndDB
//!
//! This module provides the extension registry and trait definitions that allow
//! developers to build custom business logic on top of SyndDB Core.

use crate::database::SyndDatabase;
use crate::types::*;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// Schema Extension Trait
// ============================================================================

/// Trait for defining database schemas
///
/// Extensions implement this trait to define their database tables, indexes,
/// and initial data. The SyndDB Core manages installation and migration.
#[async_trait]
pub trait SchemaExtension: Send + Sync {
    /// Unique identifier for this schema
    fn schema_id(&self) -> &str;

    /// Schema version number
    fn version(&self) -> u32;

    /// SQL statements to create tables
    fn create_statements(&self) -> Vec<String>;

    /// SQL statements to migrate from a previous version
    fn migrate_statements(&self, from_version: u32) -> Result<Vec<String>>;

    /// SQL statements to create indexes
    fn index_statements(&self) -> Vec<String>;

    /// SQL statements to seed initial data
    fn seed_statements(&self) -> Vec<String>;
}

// ============================================================================
// Local Write Extension Trait
// ============================================================================

/// Trait for defining custom write operations
///
/// Extensions implement this trait to define how their write operations
/// are validated and converted to SQL. The Core handles execution.
#[async_trait]
pub trait LocalWriteExtension: Send + Sync {
    /// Type identifier for this write operation
    fn write_type(&self) -> &str;

    /// JSON schema for validating requests
    fn schema(&self) -> &serde_json::Value;

    /// Validate a write request
    fn validate(&self, request: &serde_json::Value) -> Result<()>;

    /// Convert request to SQL statements
    fn to_sql(&self, request: &serde_json::Value) -> Result<Vec<SqlOperation>>;

    /// Pre-execution hook (optional)
    async fn pre_execute(&self, _request: &serde_json::Value) -> Result<()> {
        Ok(())
    }

    /// Post-execution hook (optional)
    async fn post_execute(
        &self,
        _request: &serde_json::Value,
        _result: &BatchResult,
    ) -> Result<()> {
        Ok(())
    }
}

// ============================================================================
// Trigger Extension Trait
// ============================================================================

/// Trait for defining SQLite triggers
///
/// Extensions implement this trait to define automated business logic
/// that runs within the database layer.
pub trait TriggerExtension: Send + Sync {
    /// Unique identifier for this trigger
    fn trigger_id(&self) -> &str;

    /// Table this trigger operates on
    fn table_name(&self) -> &str;

    /// When the trigger fires
    fn trigger_event(&self) -> TriggerEvent;

    /// SQL body of the trigger
    fn trigger_sql(&self) -> String;

    /// Other triggers this depends on (for ordering)
    fn dependencies(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Trigger timing events
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerEvent {
    BeforeInsert,
    AfterInsert,
    BeforeUpdate,
    AfterUpdate,
    BeforeDelete,
    AfterDelete,
}

impl TriggerEvent {
    pub fn to_sql(&self) -> &'static str {
        match self {
            TriggerEvent::BeforeInsert => "BEFORE INSERT",
            TriggerEvent::AfterInsert => "AFTER INSERT",
            TriggerEvent::BeforeUpdate => "BEFORE UPDATE",
            TriggerEvent::AfterUpdate => "AFTER UPDATE",
            TriggerEvent::BeforeDelete => "BEFORE DELETE",
            TriggerEvent::AfterDelete => "AFTER DELETE",
        }
    }
}

// ============================================================================
// Bridge Extension Trait
// ============================================================================

/// Trait for defining bridge operations
///
/// Extensions implement this trait to define how assets move between
/// SyndDB and external blockchains (deposits and withdrawals).
#[async_trait]
pub trait BridgeExtension: Send + Sync {
    /// Unique identifier for this bridge
    fn bridge_id(&self) -> &str;

    /// Chain ID this bridge connects to
    fn chain_id(&self) -> u64;

    /// Process a deposit from the external chain
    async fn process_deposit(&self, deposit: BridgeDeposit) -> Result<Vec<SqlOperation>>;

    /// Process a withdrawal request to the external chain
    async fn process_withdrawal(&self, withdrawal: BridgeWithdrawal) -> Result<BridgeTransaction>;

    /// Verify a bridge transaction
    async fn verify_transaction(&self, tx_hash: &str) -> Result<bool>;
}

/// Bridge deposit information
#[derive(Debug, Clone)]
pub struct BridgeDeposit {
    /// Transaction hash on source chain
    pub tx_hash: String,
    /// Account ID to credit
    pub account_id: String,
    /// Token address
    pub token_address: String,
    /// Amount deposited
    pub amount: u64,
    /// Block number
    pub block_number: u64,
}

/// Bridge withdrawal information
#[derive(Debug, Clone)]
pub struct BridgeWithdrawal {
    /// Request ID
    pub request_id: String,
    /// Account ID
    pub account_id: String,
    /// Destination address
    pub destination_address: String,
    /// Token address
    pub token_address: String,
    /// Amount to withdraw
    pub amount: u64,
}

/// Bridge transaction result
#[derive(Debug, Clone)]
pub struct BridgeTransaction {
    /// Transaction hash
    pub tx_hash: String,
    /// Gas used
    pub gas_used: u64,
    /// Status
    pub status: BridgeTransactionStatus,
}

/// Bridge transaction status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTransactionStatus {
    Pending,
    Confirmed,
    Failed,
}

// ============================================================================
// Query Extension Trait
// ============================================================================

/// Trait for defining custom query patterns
///
/// Extensions implement this trait to define commonly used queries
/// with optimized caching and execution strategies.
#[async_trait]
pub trait QueryExtension: Send + Sync {
    /// Query identifier
    fn query_id(&self) -> &str;

    /// Generate SQL for this query
    fn to_sql(&self, params: &serde_json::Value) -> Result<String>;

    /// Cache TTL in seconds (None = no caching)
    fn cache_ttl(&self) -> Option<u64> {
        None
    }

    /// Whether this query can be cached
    fn cacheable(&self) -> bool {
        self.cache_ttl().is_some()
    }
}

// ============================================================================
// Extension Registry
// ============================================================================

/// Central registry for all extensions
pub struct ExtensionRegistry {
    schemas: HashMap<String, Arc<dyn SchemaExtension>>,
    writes: HashMap<String, Arc<dyn LocalWriteExtension>>,
    triggers: HashMap<String, Arc<dyn TriggerExtension>>,
    bridges: HashMap<String, Arc<dyn BridgeExtension>>,
    queries: HashMap<String, Arc<dyn QueryExtension>>,
}

impl ExtensionRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            schemas: HashMap::new(),
            writes: HashMap::new(),
            triggers: HashMap::new(),
            bridges: HashMap::new(),
            queries: HashMap::new(),
        }
    }

    /// Register a schema extension
    pub fn register_schema(&mut self, extension: Arc<dyn SchemaExtension>) -> Result<()> {
        let id = extension.schema_id().to_string();
        if self.schemas.contains_key(&id) {
            return Err(Error::Extension(format!(
                "Schema {} already registered",
                id
            )));
        }
        self.schemas.insert(id, extension);
        Ok(())
    }

    /// Register a write extension
    pub fn register_write(&mut self, extension: Arc<dyn LocalWriteExtension>) -> Result<()> {
        let write_type = extension.write_type().to_string();
        if self.writes.contains_key(&write_type) {
            return Err(Error::Extension(format!(
                "Write type {} already registered",
                write_type
            )));
        }
        self.writes.insert(write_type, extension);
        Ok(())
    }

    /// Register a trigger extension
    pub fn register_trigger(&mut self, extension: Arc<dyn TriggerExtension>) -> Result<()> {
        let id = extension.trigger_id().to_string();
        if self.triggers.contains_key(&id) {
            return Err(Error::Extension(format!(
                "Trigger {} already registered",
                id
            )));
        }
        self.triggers.insert(id, extension);
        Ok(())
    }

    /// Register a bridge extension
    pub fn register_bridge(&mut self, extension: Arc<dyn BridgeExtension>) -> Result<()> {
        let id = extension.bridge_id().to_string();
        if self.bridges.contains_key(&id) {
            return Err(Error::Extension(format!(
                "Bridge {} already registered",
                id
            )));
        }
        self.bridges.insert(id, extension);
        Ok(())
    }

    /// Register a query extension
    pub fn register_query(&mut self, extension: Arc<dyn QueryExtension>) -> Result<()> {
        let id = extension.query_id().to_string();
        if self.queries.contains_key(&id) {
            return Err(Error::Extension(format!(
                "Query {} already registered",
                id
            )));
        }
        self.queries.insert(id, extension);
        Ok(())
    }

    /// Get a write extension by type
    pub fn get_write_extension(&self, write_type: &str) -> Option<Arc<dyn LocalWriteExtension>> {
        self.writes.get(write_type).cloned()
    }

    /// Get a query extension by ID
    pub fn get_query_extension(&self, query_id: &str) -> Option<Arc<dyn QueryExtension>> {
        self.queries.get(query_id).cloned()
    }

    /// Get a bridge extension by ID
    pub fn get_bridge_extension(&self, bridge_id: &str) -> Option<Arc<dyn BridgeExtension>> {
        self.bridges.get(bridge_id).cloned()
    }

    /// Initialize all registered extensions
    pub async fn initialize(&self, database: &dyn SyndDatabase) -> Result<()> {
        tracing::info!("Initializing {} schema extensions", self.schemas.len());

        // Install schemas
        for schema in self.schemas.values() {
            tracing::debug!("Installing schema: {}", schema.schema_id());

            // Create tables
            for statement in schema.create_statements() {
                database.execute(&statement, vec![]).await?;
            }

            // Create indexes
            for statement in schema.index_statements() {
                database.execute(&statement, vec![]).await?;
            }

            // Seed data
            for statement in schema.seed_statements() {
                database.execute(&statement, vec![]).await?;
            }
        }

        // Install triggers (with dependency ordering)
        let ordered_triggers = self.order_triggers_by_dependencies()?;
        tracing::info!("Installing {} triggers", ordered_triggers.len());

        for trigger in ordered_triggers {
            tracing::debug!("Installing trigger: {}", trigger.trigger_id());

            let sql = format!(
                "CREATE TRIGGER IF NOT EXISTS {} {} ON {} BEGIN {} END",
                trigger.trigger_id(),
                trigger.trigger_event().to_sql(),
                trigger.table_name(),
                trigger.trigger_sql()
            );

            database.execute(&sql, vec![]).await?;
        }

        tracing::info!("Extension initialization complete");
        Ok(())
    }

    /// Order triggers by their dependencies
    fn order_triggers_by_dependencies(&self) -> Result<Vec<Arc<dyn TriggerExtension>>> {
        let mut ordered = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut visiting = std::collections::HashSet::new();

        // Simple topological sort
        for trigger in self.triggers.values() {
            self.visit_trigger(
                trigger.clone(),
                &mut ordered,
                &mut visited,
                &mut visiting,
            )?;
        }

        Ok(ordered)
    }

    /// Visit a trigger in dependency order (DFS)
    fn visit_trigger(
        &self,
        trigger: Arc<dyn TriggerExtension>,
        ordered: &mut Vec<Arc<dyn TriggerExtension>>,
        visited: &mut std::collections::HashSet<String>,
        visiting: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        let id = trigger.trigger_id().to_string();

        if visited.contains(&id) {
            return Ok(());
        }

        if visiting.contains(&id) {
            return Err(Error::Extension(format!(
                "Circular trigger dependency detected: {}",
                id
            )));
        }

        visiting.insert(id.clone());

        // Visit dependencies first
        for dep_id in trigger.dependencies() {
            if let Some(dep_trigger) = self.triggers.get(&dep_id) {
                self.visit_trigger(dep_trigger.clone(), ordered, visited, visiting)?;
            }
        }

        visiting.remove(&id);
        visited.insert(id);
        ordered.push(trigger);

        Ok(())
    }

    /// Get statistics about registered extensions
    pub fn stats(&self) -> ExtensionStats {
        ExtensionStats {
            schema_count: self.schemas.len(),
            write_count: self.writes.len(),
            trigger_count: self.triggers.len(),
            bridge_count: self.bridges.len(),
            query_count: self.queries.len(),
        }
    }
}

impl Default for ExtensionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about registered extensions
#[derive(Debug, Clone)]
pub struct ExtensionStats {
    pub schema_count: usize,
    pub write_count: usize,
    pub trigger_count: usize,
    pub bridge_count: usize,
    pub query_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestSchema;

    #[async_trait]
    impl SchemaExtension for TestSchema {
        fn schema_id(&self) -> &str {
            "test"
        }

        fn version(&self) -> u32 {
            1
        }

        fn create_statements(&self) -> Vec<String> {
            vec!["CREATE TABLE test (id INTEGER PRIMARY KEY)".to_string()]
        }

        fn migrate_statements(&self, _from_version: u32) -> Result<Vec<String>> {
            Ok(Vec::new())
        }

        fn index_statements(&self) -> Vec<String> {
            Vec::new()
        }

        fn seed_statements(&self) -> Vec<String> {
            Vec::new()
        }
    }

    #[test]
    fn test_extension_registry() {
        let mut registry = ExtensionRegistry::new();
        let schema = Arc::new(TestSchema);

        assert!(registry.register_schema(schema.clone()).is_ok());

        // Duplicate registration should fail
        assert!(registry.register_schema(schema).is_err());

        let stats = registry.stats();
        assert_eq!(stats.schema_count, 1);
    }
}
