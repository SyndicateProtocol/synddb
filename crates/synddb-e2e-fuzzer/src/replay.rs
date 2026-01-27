//! Deterministic replay for E2E fuzzing
//!
//! Provides seed-based scenario generation for reproducible test cases.

use crate::scenarios::{E2EAction, E2EScenario};
use rand::prelude::*;
use synddb_fuzzer::{DmlOperation, SqlValue};

/// Generate a deterministic scenario from a seed
///
/// This allows reproducing exact test cases from CI failures.
pub fn scenario_from_seed(seed: u64) -> E2EScenario {
    let mut rng = StdRng::seed_from_u64(seed);
    let table = "test_table".to_string();

    // Generate 5-30 operations
    let num_ops = rng.random_range(5..30);
    let mut actions = Vec::new();

    for _ in 0..num_ops {
        let action_type = rng.random_range(0..10);

        let action = if action_type < 4 {
            // UPDATE (40%)
            let id = rng.random_range(1..50);
            let value = rng.random_range(-1000..1000);
            E2EAction::ExecuteDml(DmlOperation::Update {
                table_name: table.clone(),
                set_column: "value".to_string(),
                set_value: SqlValue::Integer(value),
                where_column: "id".to_string(),
                where_value: SqlValue::Integer(id),
            })
        } else if action_type < 7 {
            // INSERT (30%)
            let id = rng.random_range(100..1000);
            let value = rng.random_range(0..1000);
            let name: String = (0..8)
                .map(|_| rng.sample(rand::distr::Alphanumeric) as char)
                .collect();
            E2EAction::ExecuteDml(DmlOperation::Insert {
                table_name: table.clone(),
                columns: vec!["id".to_string(), "value".to_string(), "name".to_string()],
                values: vec![
                    SqlValue::Integer(id),
                    SqlValue::Integer(value),
                    SqlValue::Text(name),
                ],
            })
        } else if action_type < 8 {
            // DELETE (10%)
            let id = rng.random_range(1..50);
            E2EAction::ExecuteDml(DmlOperation::Delete {
                table_name: table.clone(),
                where_column: "id".to_string(),
                where_value: SqlValue::Integer(id),
            })
        } else {
            // SYNC (20%)
            E2EAction::SyncValidator
        };

        actions.push(action);
    }

    // Always end with a sync
    if !matches!(actions.last(), Some(E2EAction::SyncValidator)) {
        actions.push(E2EAction::SyncValidator);
    }

    E2EScenario {
        schema: format!(
            "CREATE TABLE {} (id INTEGER PRIMARY KEY, value INTEGER, name TEXT)",
            table
        ),
        initial_data: (1..=10)
            .map(|i| format!("INSERT INTO {} VALUES ({}, 0, 'user{}')", table, i, i))
            .collect(),
        actions,
    }
}

/// Get the current seed from environment variable
pub fn get_seed_from_env() -> Option<u64> {
    std::env::var("SYNDDB_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
}

/// Run a single deterministic test case
pub fn run_seeded_test(seed: u64) -> anyhow::Result<()> {
    use crate::{
        harness::E2EHarness, invariants::check_all_invariants, scenarios::execute_scenario,
    };

    let scenario = scenario_from_seed(seed);

    eprintln!("Running scenario with seed {}", seed);
    eprintln!("  Actions: {}", scenario.actions.len());

    let mut harness = E2EHarness::new()?;
    execute_scenario(&mut harness, &scenario)?;

    let violations = check_all_invariants(&harness);
    if !violations.is_empty() {
        anyhow::bail!("Invariant violations with seed {}: {:?}", seed, violations);
    }

    eprintln!("  PASSED");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seed_reproducibility() {
        let seed = 12345u64;

        // Generate twice with same seed
        let scenario1 = scenario_from_seed(seed);
        let scenario2 = scenario_from_seed(seed);

        // Should be identical
        assert_eq!(scenario1.schema, scenario2.schema);
        assert_eq!(scenario1.initial_data.len(), scenario2.initial_data.len());
        assert_eq!(scenario1.actions.len(), scenario2.actions.len());
    }

    #[test]
    fn test_different_seeds_different_scenarios() {
        let scenario1 = scenario_from_seed(1);
        let scenario2 = scenario_from_seed(2);

        // Should be different (with very high probability)
        assert_ne!(scenario1.actions.len(), scenario2.actions.len());
    }

    #[test]
    fn test_scenario_generation() {
        let scenario = scenario_from_seed(42);

        assert!(!scenario.schema.is_empty());
        assert!(!scenario.initial_data.is_empty());
        assert!(!scenario.actions.is_empty());

        // Should end with sync
        assert!(matches!(
            scenario.actions.last(),
            Some(E2EAction::SyncValidator)
        ));
    }

    #[test]
    fn test_seeded_execution() {
        // Run a few seeded tests
        for seed in [1, 42, 12345] {
            run_seeded_test(seed).unwrap_or_else(|e| panic!("Seed {} failed: {}", seed, e));
        }
    }
}
