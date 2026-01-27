//! Custom validation rules for changesets
//!
//! This module provides the extension points for custom validation rules.
//! Rules are applied after a changeset is applied but before it's committed.
//! If any rule fails, the changeset is rejected and the transaction is rolled back.
//!
//! # Implementing Custom Rules
//!
//! To create a custom validator with your own rules:
//!
//! 1. Create a new crate that depends on `synddb-validator`
//! 2. Implement the `ValidationRule` trait for your rules
//! 3. Register rules with a `RuleRegistry`
//! 4. Pass the registry to the validator
//!
//! # Example
//!
//! ```rust,ignore
//! use synddb_validator::rules::{ValidationRule, ValidationResult, RuleRegistry};
//! use rusqlite::Connection;
//!
//! struct MyCustomRule {
//!     threshold: u32,
//! }
//!
//! impl ValidationRule for MyCustomRule {
//!     fn name(&self) -> &str { "my_custom_rule" }
//!
//!     fn validate(&self, conn: &Connection, sequence: u64) -> anyhow::Result<ValidationResult> {
//!         // Query database and validate
//!         Ok(ValidationResult::Pass)
//!     }
//! }
//!
//! // In your custom validator:
//! let mut registry = RuleRegistry::new();
//! registry.register(Box::new(MyCustomRule { threshold: 100 }));
//! ```

use anyhow::Result;
use rusqlite::Connection;
use tracing::{debug, trace, warn};

use crate::error::ValidatorError;

/// Result of a validation rule check
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationResult {
    /// Validation passed
    Pass,
    /// Validation failed with reason
    Fail {
        /// Human-readable explanation of why validation failed
        reason: String,
    },
    /// Rule does not apply to this changeset (e.g., wrong table)
    NotApplicable,
}

/// Trait for custom validation rules
///
/// Rules inspect the database state AFTER a changeset has been applied
/// (within a transaction) and decide whether to accept or reject it.
///
/// # Implementation Notes
///
/// - The connection passed to `validate()` is within an uncommitted transaction
/// - If validation returns `Fail`, the transaction will be rolled back
/// - Rules should be deterministic - same database state should always produce same result
/// - Rules should be fast - they run on every changeset
pub trait ValidationRule: Send + Sync {
    /// Unique identifier for this rule
    fn name(&self) -> &str;

    /// Validate the current database state after changeset application
    ///
    /// The connection is within an uncommitted transaction. If this returns
    /// `Fail`, the transaction will be rolled back.
    ///
    /// # Arguments
    ///
    /// * `conn` - Database connection with uncommitted changeset applied
    /// * `sequence` - The sequence number of the changeset being validated
    ///
    /// # Returns
    ///
    /// * `Pass` - Validation succeeded, changeset can be committed
    /// * `Fail` - Validation failed, changeset should be rejected
    /// * `NotApplicable` - Rule doesn't apply to this changeset (e.g., wrong table)
    fn validate(&self, conn: &Connection, sequence: u64) -> Result<ValidationResult>;

    /// Whether this rule is enabled
    ///
    /// Disabled rules are skipped during validation. This allows rules to be
    /// toggled via configuration without removing them from the registry.
    fn is_enabled(&self) -> bool {
        true
    }
}

/// Registry of validation rules
///
/// The registry holds all active validation rules and provides a method to
/// run all of them against the current database state.
#[derive(Default)]
pub struct RuleRegistry {
    rules: Vec<Box<dyn ValidationRule>>,
}

impl RuleRegistry {
    /// Create a new empty rule registry
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Register a new validation rule
    ///
    /// Rules are run in the order they are registered.
    pub fn register(&mut self, rule: Box<dyn ValidationRule>) {
        tracing::info!(rule = rule.name(), "Registered validation rule");
        self.rules.push(rule);
    }

    /// Get the number of registered rules
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Run all applicable rules against the current database state
    ///
    /// # Arguments
    ///
    /// * `conn` - Database connection with uncommitted changeset applied
    /// * `sequence` - The sequence number being validated
    ///
    /// # Returns
    ///
    /// * `Ok(())` - All rules passed or were not applicable
    /// * `Err(ValidatorError::ValidationRuleFailed)` - A rule failed validation
    pub fn validate_all(&self, conn: &Connection, sequence: u64) -> Result<(), ValidatorError> {
        for rule in &self.rules {
            if !rule.is_enabled() {
                trace!(rule = rule.name(), sequence, "Rule disabled, skipping");
                continue;
            }

            match rule.validate(conn, sequence) {
                Ok(ValidationResult::Pass) => {
                    debug!(rule = rule.name(), sequence, "Rule passed");
                }
                Ok(ValidationResult::Fail { reason }) => {
                    warn!(
                        rule = rule.name(),
                        sequence,
                        reason = %reason,
                        "Validation rule failed"
                    );
                    return Err(ValidatorError::ValidationRuleFailed {
                        rule: rule.name().to_string(),
                        sequence,
                        reason,
                    });
                }
                Ok(ValidationResult::NotApplicable) => {
                    trace!(rule = rule.name(), sequence, "Rule not applicable");
                }
                Err(e) => {
                    warn!(
                        rule = rule.name(),
                        sequence,
                        error = %e,
                        "Rule validation error"
                    );
                    return Err(ValidatorError::ValidationRuleFailed {
                        rule: rule.name().to_string(),
                        sequence,
                        reason: format!("Rule error: {e}"),
                    });
                }
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for RuleRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleRegistry")
            .field("rule_count", &self.rules.len())
            .field(
                "rules",
                &self.rules.iter().map(|r| r.name()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AlwaysPassRule;

    impl ValidationRule for AlwaysPassRule {
        fn name(&self) -> &str {
            "always_pass"
        }

        fn validate(&self, _conn: &Connection, _sequence: u64) -> Result<ValidationResult> {
            Ok(ValidationResult::Pass)
        }
    }

    struct AlwaysFailRule {
        reason: String,
    }

    impl ValidationRule for AlwaysFailRule {
        fn name(&self) -> &str {
            "always_fail"
        }

        fn validate(&self, _conn: &Connection, _sequence: u64) -> Result<ValidationResult> {
            Ok(ValidationResult::Fail {
                reason: self.reason.clone(),
            })
        }
    }

    struct DisabledRule;

    impl ValidationRule for DisabledRule {
        fn name(&self) -> &str {
            "disabled"
        }

        fn validate(&self, _conn: &Connection, _sequence: u64) -> Result<ValidationResult> {
            panic!("Should not be called when disabled");
        }

        fn is_enabled(&self) -> bool {
            false
        }
    }

    #[test]
    fn test_empty_registry() {
        let registry = RuleRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);

        let conn = Connection::open_in_memory().unwrap();
        assert!(registry.validate_all(&conn, 1).is_ok());
    }

    #[test]
    fn test_passing_rule() {
        let mut registry = RuleRegistry::new();
        registry.register(Box::new(AlwaysPassRule));

        let conn = Connection::open_in_memory().unwrap();
        assert!(registry.validate_all(&conn, 1).is_ok());
    }

    #[test]
    fn test_failing_rule() {
        let mut registry = RuleRegistry::new();
        registry.register(Box::new(AlwaysFailRule {
            reason: "test failure".to_string(),
        }));

        let conn = Connection::open_in_memory().unwrap();
        let result = registry.validate_all(&conn, 42);
        assert!(result.is_err());

        match result.unwrap_err() {
            ValidatorError::ValidationRuleFailed {
                rule,
                sequence,
                reason,
            } => {
                assert_eq!(rule, "always_fail");
                assert_eq!(sequence, 42);
                assert_eq!(reason, "test failure");
            }
            e => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_disabled_rule_skipped() {
        let mut registry = RuleRegistry::new();
        registry.register(Box::new(DisabledRule));

        let conn = Connection::open_in_memory().unwrap();
        // Should not panic because disabled rule is skipped
        assert!(registry.validate_all(&conn, 1).is_ok());
    }

    #[test]
    fn test_multiple_rules_order() {
        let mut registry = RuleRegistry::new();
        registry.register(Box::new(AlwaysPassRule));
        registry.register(Box::new(AlwaysFailRule {
            reason: "second rule fails".to_string(),
        }));

        assert_eq!(registry.len(), 2);

        let conn = Connection::open_in_memory().unwrap();
        let result = registry.validate_all(&conn, 1);
        assert!(result.is_err());
    }
}
