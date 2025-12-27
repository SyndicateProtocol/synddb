//! E2E test scenarios and proptest strategies
//!
//! Defines scenario types and proptest strategies for generating random
//! E2E test cases.

use proptest::prelude::*;
use synddb_fuzzer::{DmlOperation, SqlValue};

/// An action in an E2E test scenario
#[derive(Debug, Clone)]
pub enum E2EAction {
    /// Execute DML and sequence the changeset
    ExecuteDml(DmlOperation),
    /// Sync the validator (process all pending messages)
    SyncValidator,
    /// Execute DDL (requires manual schema sync for now)
    ExecuteDdl(String),
}

/// A complete E2E test scenario
#[derive(Debug, Clone)]
pub struct E2EScenario {
    /// Initial schema (CREATE TABLE statements)
    pub schema: String,
    /// Initial data to populate (INSERT statements)
    pub initial_data: Vec<String>,
    /// Sequence of actions to perform
    pub actions: Vec<E2EAction>,
}

impl E2EScenario {
    /// Create a simple scenario with a single table
    pub fn single_table(table_name: &str, operations: Vec<DmlOperation>) -> Self {
        let schema = format!(
            "CREATE TABLE {} (id INTEGER PRIMARY KEY, value INTEGER, name TEXT)",
            table_name
        );

        let initial_data = vec![format!(
            "INSERT INTO {} VALUES (1, 0, 'initial')",
            table_name
        )];

        let actions = operations.into_iter().map(E2EAction::ExecuteDml).collect();

        Self {
            schema,
            initial_data,
            actions,
        }
    }
}

/// Generate a table name
fn table_name_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{2,8}".prop_map(|s| s.to_lowercase())
}

/// Generate a column name
fn column_name_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{1,6}".prop_map(|s| s.to_lowercase())
}

/// Generate a SQL value suitable for testing
fn sql_value_strategy() -> impl Strategy<Value = SqlValue> {
    prop_oneof![
        3 => any::<i64>().prop_map(SqlValue::Integer),
        2 => "[a-zA-Z0-9 ]{0,20}".prop_map(SqlValue::Text),
        1 => Just(SqlValue::Null),
    ]
}

/// Strategy for generating UPDATE operations
fn update_operation_strategy(table: String) -> impl Strategy<Value = DmlOperation> {
    (1i64..100, sql_value_strategy()).prop_map(move |(id, value)| DmlOperation::Update {
        table_name: table.clone(),
        set_column: "value".to_string(),
        set_value: value,
        where_column: "id".to_string(),
        where_value: SqlValue::Integer(id),
    })
}

/// Strategy for generating INSERT operations
fn insert_operation_strategy(table: String) -> impl Strategy<Value = DmlOperation> {
    (100i64..1000, sql_value_strategy(), "[a-zA-Z]{1,10}").prop_map(move |(id, value, name)| {
        let value_int = match &value {
            SqlValue::Integer(i) => *i,
            _ => 0,
        };
        DmlOperation::Insert {
            table_name: table.clone(),
            columns: vec!["id".to_string(), "value".to_string(), "name".to_string()],
            values: vec![
                SqlValue::Integer(id),
                SqlValue::Integer(value_int),
                SqlValue::Text(name),
            ],
        }
    })
}

/// Strategy for generating DELETE operations
fn delete_operation_strategy(table: String) -> impl Strategy<Value = DmlOperation> {
    (1i64..100).prop_map(move |id| DmlOperation::Delete {
        table_name: table.clone(),
        where_column: "id".to_string(),
        where_value: SqlValue::Integer(id),
    })
}

/// Strategy for generating any DML operation
fn dml_operation_strategy(table: String) -> impl Strategy<Value = DmlOperation> {
    prop_oneof![
        4 => update_operation_strategy(table.clone()),
        3 => insert_operation_strategy(table.clone()),
        1 => delete_operation_strategy(table),
    ]
}

/// Strategy for generating E2E actions
fn e2e_action_strategy(table: String) -> impl Strategy<Value = E2EAction> {
    prop_oneof![
        8 => dml_operation_strategy(table).prop_map(E2EAction::ExecuteDml),
        2 => Just(E2EAction::SyncValidator),
    ]
}

/// Strategy for generating simple E2E scenarios
pub fn simple_scenario_strategy() -> impl Strategy<Value = E2EScenario> {
    let table = "test_table".to_string();

    prop::collection::vec(e2e_action_strategy(table.clone()), 1..20).prop_map(move |actions| {
        E2EScenario {
            schema: format!(
                "CREATE TABLE {} (id INTEGER PRIMARY KEY, value INTEGER, name TEXT)",
                table
            ),
            initial_data: vec![
                format!("INSERT INTO {} VALUES (1, 0, 'alice')", table),
                format!("INSERT INTO {} VALUES (2, 0, 'bob')", table),
                format!("INSERT INTO {} VALUES (3, 0, 'charlie')", table),
            ],
            actions,
        }
    })
}

/// Strategy for generating E2E scenarios with multiple syncs
pub fn multi_sync_scenario_strategy() -> impl Strategy<Value = E2EScenario> {
    let table = "test_table".to_string();

    // Generate actions with guaranteed sync points
    prop::collection::vec(dml_operation_strategy(table.clone()), 2..10).prop_map(move |ops| {
        let mut actions = Vec::new();
        for (i, op) in ops.into_iter().enumerate() {
            actions.push(E2EAction::ExecuteDml(op));
            // Sync every 3 operations
            if (i + 1) % 3 == 0 {
                actions.push(E2EAction::SyncValidator);
            }
        }
        // Always end with a sync
        actions.push(E2EAction::SyncValidator);

        E2EScenario {
            schema: format!(
                "CREATE TABLE {} (id INTEGER PRIMARY KEY, value INTEGER, name TEXT)",
                table
            ),
            initial_data: vec![
                format!("INSERT INTO {} VALUES (1, 0, 'alice')", table),
                format!("INSERT INTO {} VALUES (2, 0, 'bob')", table),
            ],
            actions,
        }
    })
}

/// Strategy for high-volume scenarios (many operations)
pub fn high_volume_scenario_strategy() -> impl Strategy<Value = E2EScenario> {
    let table = "test_table".to_string();

    prop::collection::vec(dml_operation_strategy(table.clone()), 50..100).prop_map(move |ops| {
        let mut actions: Vec<E2EAction> = ops.into_iter().map(E2EAction::ExecuteDml).collect();
        actions.push(E2EAction::SyncValidator);

        E2EScenario {
            schema: format!(
                "CREATE TABLE {} (id INTEGER PRIMARY KEY, value INTEGER, name TEXT)",
                table
            ),
            initial_data: (1..=50)
                .map(|i| format!("INSERT INTO {} VALUES ({}, 0, 'user{}')", table, i, i))
                .collect(),
            actions,
        }
    })
}

/// Strategy for stress testing with many operations and frequent syncs
pub fn stress_scenario_strategy() -> impl Strategy<Value = E2EScenario> {
    let table = "test_table".to_string();

    prop::collection::vec(dml_operation_strategy(table.clone()), 200..500).prop_map(move |ops| {
        let mut actions = Vec::new();
        for (i, op) in ops.into_iter().enumerate() {
            actions.push(E2EAction::ExecuteDml(op));
            // Sync at varied intervals (prime numbers to avoid patterns)
            if i % 17 == 0 || i % 23 == 0 {
                actions.push(E2EAction::SyncValidator);
            }
        }
        actions.push(E2EAction::SyncValidator);

        E2EScenario {
            schema: format!(
                "CREATE TABLE {} (id INTEGER PRIMARY KEY, value INTEGER, name TEXT)",
                table
            ),
            initial_data: (1..=100)
                .map(|i| format!("INSERT INTO {} VALUES ({}, 0, 'user{}')", table, i, i))
                .collect(),
            actions,
        }
    })
}

/// Strategy for testing with large TEXT values (near `SQLite` limits)
pub fn large_payload_scenario_strategy() -> impl Strategy<Value = E2EScenario> {
    let table = "test_table".to_string();

    // Generate operations with large text values
    prop::collection::vec(
        (1i64..50, "[a-zA-Z0-9]{500,1000}").prop_map(|(id, large_text)| DmlOperation::Update {
            table_name: "test_table".to_string(),
            set_column: "name".to_string(),
            set_value: SqlValue::Text(large_text),
            where_column: "id".to_string(),
            where_value: SqlValue::Integer(id),
        }),
        10..30,
    )
    .prop_map(move |ops| {
        let mut actions: Vec<E2EAction> = ops.into_iter().map(E2EAction::ExecuteDml).collect();
        actions.push(E2EAction::SyncValidator);

        E2EScenario {
            schema: format!(
                "CREATE TABLE {} (id INTEGER PRIMARY KEY, value INTEGER, name TEXT)",
                table
            ),
            initial_data: (1..=50)
                .map(|i| format!("INSERT INTO {} VALUES ({}, 0, 'user{}')", table, i, i))
                .collect(),
            actions,
        }
    })
}

/// Strategy for rapid insert/delete cycles (tests tombstone handling)
pub fn churn_scenario_strategy() -> impl Strategy<Value = E2EScenario> {
    let table = "test_table".to_string();

    // Generate alternating inserts and deletes
    prop::collection::vec(1000i64..2000, 20..50).prop_map(move |ids| {
        let mut actions = Vec::new();

        for id in ids {
            // Insert
            actions.push(E2EAction::ExecuteDml(DmlOperation::Insert {
                table_name: table.clone(),
                columns: vec!["id".to_string(), "value".to_string(), "name".to_string()],
                values: vec![
                    SqlValue::Integer(id),
                    SqlValue::Integer(0),
                    SqlValue::Text("temp".to_string()),
                ],
            }));
            // Delete
            actions.push(E2EAction::ExecuteDml(DmlOperation::Delete {
                table_name: table.clone(),
                where_column: "id".to_string(),
                where_value: SqlValue::Integer(id),
            }));
        }
        actions.push(E2EAction::SyncValidator);

        E2EScenario {
            schema: format!(
                "CREATE TABLE {} (id INTEGER PRIMARY KEY, value INTEGER, name TEXT)",
                table
            ),
            initial_data: vec![format!("INSERT INTO {} VALUES (1, 0, 'anchor')", table)],
            actions,
        }
    })
}

/// Execute a scenario on a harness
pub fn execute_scenario(
    harness: &mut crate::harness::E2EHarness,
    scenario: &E2EScenario,
) -> anyhow::Result<()> {
    use anyhow::Context;

    // Setup schema
    harness
        .setup_schema(&scenario.schema)
        .context("Failed to setup schema")?;

    // Insert initial data on both databases
    for sql in &scenario.initial_data {
        harness
            .source()
            .execute_batch(sql)
            .context("Failed to insert initial data on source")?;
        harness
            .replica()
            .execute_batch(sql)
            .context("Failed to insert initial data on replica")?;
    }

    // Execute actions
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("Failed to create runtime")?;

    for action in &scenario.actions {
        match action {
            E2EAction::ExecuteDml(op) => {
                let sql = op.to_sql();
                // Try to execute - may fail due to constraint violations
                match harness.execute_and_sequence(&sql) {
                    Ok(_) => {}
                    Err(e) => {
                        // Constraint violations are expected - check full error chain
                        let err_str = format!("{:#}", e);
                        if !err_str.contains("UNIQUE constraint")
                            && !err_str.contains("NOT NULL constraint")
                            && !err_str.contains("FOREIGN KEY constraint")
                            && !err_str.contains("no such table")
                            && !err_str.contains("changeset is empty")
                        // No changes made
                        {
                            return Err(e);
                        }
                    }
                }
            }
            E2EAction::SyncValidator => {
                rt.block_on(harness.sync_validator())
                    .context("Failed to sync validator")?;
            }
            E2EAction::ExecuteDdl(sql) => {
                harness.execute_ddl(sql).context("Failed to execute DDL")?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10))]

        #[test]
        fn test_simple_scenario_generation(scenario in simple_scenario_strategy()) {
            prop_assert!(!scenario.schema.is_empty());
            prop_assert!(!scenario.initial_data.is_empty());
            prop_assert!(!scenario.actions.is_empty());
        }

        #[test]
        fn test_multi_sync_scenario_generation(scenario in multi_sync_scenario_strategy()) {
            // Should have at least one sync
            let sync_count = scenario.actions.iter()
                .filter(|a| matches!(a, E2EAction::SyncValidator))
                .count();
            prop_assert!(sync_count >= 1, "Expected at least 1 sync, got {}", sync_count);
        }
    }

    #[test]
    fn test_single_table_scenario() {
        let ops = vec![DmlOperation::Update {
            table_name: "test".to_string(),
            set_column: "value".to_string(),
            set_value: SqlValue::Integer(42),
            where_column: "id".to_string(),
            where_value: SqlValue::Integer(1),
        }];

        let scenario = E2EScenario::single_table("test", ops);
        assert!(scenario.schema.contains("CREATE TABLE test"));
        assert!(!scenario.initial_data.is_empty());
        assert_eq!(scenario.actions.len(), 1);
    }
}
