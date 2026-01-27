//! Property-based tests for `SQLite` changeset capture and replay
//!
//! This module uses proptest to generate random SQL operations and verify
//! that `SyndDB` correctly captures and replays them.

use crate::{
    generators::FuzzScenario,
    replay::{snapshot_database, ReplayResult},
};
use anyhow::{Context, Result};
use rusqlite::{session::Session, Connection};
use std::io::Cursor;

/// Execute a scenario and capture changesets using `SQLite`'s session extension
pub fn execute_with_changeset_capture(scenario: &FuzzScenario) -> Result<(Connection, Vec<u8>)> {
    let conn = Connection::open_in_memory()?;

    // Execute schema first (before session attachment)
    for sql in scenario.schema_sql() {
        conn.execute(&sql, [])
            .with_context(|| format!("Failed to execute schema SQL: {}", sql))?;
    }

    // Capture changeset in a block so session is dropped before we return conn
    let changeset = {
        // Create session and attach to all tables
        let mut session = Session::new(&conn).context("Failed to create session")?;
        session
            .attach(None::<&str>)
            .context("Failed to attach session")?;

        // Execute operations
        for sql in scenario.operation_sql() {
            // Some operations may fail (e.g., UPDATE on empty table), which is OK
            let _ = conn.execute(&sql, []);
        }

        // Extract changeset
        let mut changeset = Vec::new();
        session
            .changeset_strm(&mut changeset)
            .context("Failed to extract changeset")?;
        changeset
    };

    Ok((conn, changeset))
}

/// Apply a changeset to a fresh database with the same schema
pub fn apply_changeset_to_fresh_db(
    scenario: &FuzzScenario,
    changeset: &[u8],
) -> Result<Connection> {
    use rusqlite::session::ConflictAction;

    let conn = Connection::open_in_memory()?;

    // Execute schema
    for sql in scenario.schema_sql() {
        conn.execute(&sql, [])
            .with_context(|| format!("Failed to execute schema SQL: {}", sql))?;
    }

    // Apply changeset if not empty
    if !changeset.is_empty() {
        let mut cursor = Cursor::new(changeset);
        conn.apply_strm(
            &mut cursor,
            None::<fn(&str) -> bool>,
            |_conflict_type, _item| ConflictAction::SQLITE_CHANGESET_ABORT,
        )
        .context("Failed to apply changeset")?;
    }

    Ok(conn)
}

/// Verify that executing operations and replaying changesets produces identical results
pub fn verify_changeset_roundtrip(scenario: &FuzzScenario) -> Result<ReplayResult> {
    // Execute scenario and capture changeset
    let (source_conn, changeset) = execute_with_changeset_capture(scenario)?;
    let source_snapshot = snapshot_database(&source_conn)?;

    // Apply changeset to fresh database
    let target_conn = apply_changeset_to_fresh_db(scenario, &changeset)?;
    let target_snapshot = snapshot_database(&target_conn)?;

    // Compare
    if source_snapshot.is_equivalent(&target_snapshot) {
        Ok(ReplayResult::Consistent)
    } else {
        Ok(ReplayResult::Inconsistent(
            source_snapshot.diff(&target_snapshot),
        ))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        generators::{fuzz_scenario_strategy, multi_table_scenario_strategy},
        operations::*,
    };
    use proptest::prelude::*;

    // ========================================================================
    // Property Tests (Proptest)
    // ========================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Test that single-table scenarios produce consistent results
        #[test]
        fn prop_single_table_changeset_roundtrip(scenario in fuzz_scenario_strategy()) {
            let result = verify_changeset_roundtrip(&scenario);
            match result {
                Ok(ReplayResult::Consistent) => {},
                Ok(ReplayResult::Inconsistent(diffs)) => {
                    // Print scenario for debugging
                    println!("Scenario schema:");
                    for sql in scenario.schema_sql() {
                        println!("  {}", sql);
                    }
                    println!("Scenario operations:");
                    for sql in scenario.operation_sql() {
                        println!("  {}", sql);
                    }
                    println!("Differences:");
                    for diff in &diffs {
                        println!("  {}", diff);
                    }
                    prop_assert!(false, "Changeset roundtrip produced inconsistent results: {:?}", diffs);
                }
                Err(e) => {
                    // Some errors are expected (e.g., constraint violations in random data)
                    // Only fail on unexpected errors
                    let err_str = e.to_string();
                    let expected_errors = [
                        "UNIQUE constraint failed",
                        "NOT NULL constraint failed",
                        "CHECK constraint failed",
                        "FOREIGN KEY constraint failed",
                        "PRIMARY KEY constraint failed",
                    ];
                    let is_expected = expected_errors.iter().any(|e| err_str.contains(e));
                    if !is_expected {
                        prop_assert!(false, "Unexpected error: {}", e);
                    }
                }
            }
        }

        /// Test that multi-table scenarios produce consistent results
        #[test]
        fn prop_multi_table_changeset_roundtrip(scenario in multi_table_scenario_strategy()) {
            let result = verify_changeset_roundtrip(&scenario);
            match result {
                Ok(ReplayResult::Consistent) => {},
                Ok(ReplayResult::Inconsistent(diffs)) => {
                    println!("Multi-table scenario schema:");
                    for sql in scenario.schema_sql() {
                        println!("  {}", sql);
                    }
                    println!("Differences:");
                    for diff in &diffs {
                        println!("  {}", diff);
                    }
                    prop_assert!(false, "Multi-table changeset roundtrip produced inconsistent results: {:?}", diffs);
                }
                Err(e) => {
                    let err_str = e.to_string();
                    let expected_errors = [
                        "UNIQUE constraint failed",
                        "NOT NULL constraint failed",
                        "CHECK constraint failed",
                        "FOREIGN KEY constraint failed",
                        "PRIMARY KEY constraint failed",
                    ];
                    let is_expected = expected_errors.iter().any(|e| err_str.contains(e));
                    if !is_expected {
                        prop_assert!(false, "Unexpected error in multi-table test: {}", e);
                    }
                }
            }
        }

        /// Test that empty operations produce empty changesets
        #[test]
        fn prop_empty_operations_empty_changeset(scenario in fuzz_scenario_strategy()) {
            // Create a scenario with schema but no operations
            let empty_scenario = FuzzScenario {
                schema: scenario.schema,
                operations: vec![],
            };

            let result = execute_with_changeset_capture(&empty_scenario);
            match result {
                Ok((_, changeset)) => {
                    prop_assert!(changeset.is_empty(), "Empty operations should produce empty changeset");
                }
                Err(e) => {
                    prop_assert!(false, "Schema-only scenario should not fail: {}", e);
                }
            }
        }

        /// Test that executing the same scenario twice produces identical results
        #[test]
        fn prop_deterministic_execution(scenario in fuzz_scenario_strategy()) {
            // Execute scenario twice
            let result1 = execute_with_changeset_capture(&scenario);
            let result2 = execute_with_changeset_capture(&scenario);

            match (result1, result2) {
                (Ok((conn1, _)), Ok((conn2, _))) => {
                    let snap1 = snapshot_database(&conn1).expect("Failed to snapshot db1");
                    let snap2 = snapshot_database(&conn2).expect("Failed to snapshot db2");
                    prop_assert!(
                        snap1.is_equivalent(&snap2),
                        "Same scenario should produce identical results"
                    );
                }
                (Err(_), Err(_)) => {
                    // Both failed, which is consistent
                }
                (Ok(_), Err(e)) | (Err(e), Ok(_)) => {
                    // One succeeded and one failed - this is a problem
                    // unless it's a constraint violation (which can be non-deterministic with random data)
                    let err_str = e.to_string();
                    let expected_errors = [
                        "UNIQUE constraint failed",
                        "NOT NULL constraint failed",
                    ];
                    let is_expected = expected_errors.iter().any(|e| err_str.contains(e));
                    if !is_expected {
                        prop_assert!(false, "Inconsistent execution: one succeeded, one failed: {}", e);
                    }
                }
            }
        }
    }

    // ========================================================================
    // Unit Tests
    // ========================================================================

    #[test]
    fn test_basic_insert_roundtrip() {
        let scenario = FuzzScenario {
            schema: vec![DdlOperation::CreateTable {
                table_name: "users".to_string(),
                columns: vec![
                    ColumnDef {
                        name: "id".to_string(),
                        col_type: ColumnType::Integer,
                        nullable: false,
                        primary_key: true,
                    },
                    ColumnDef {
                        name: "name".to_string(),
                        col_type: ColumnType::Text,
                        nullable: true,
                        primary_key: false,
                    },
                ],
                if_not_exists: true,
            }],
            operations: vec![
                DmlOperation::Insert {
                    table_name: "users".to_string(),
                    columns: vec!["id".to_string(), "name".to_string()],
                    values: vec![SqlValue::Integer(1), SqlValue::Text("Alice".to_string())],
                },
                DmlOperation::Insert {
                    table_name: "users".to_string(),
                    columns: vec!["id".to_string(), "name".to_string()],
                    values: vec![SqlValue::Integer(2), SqlValue::Text("Bob".to_string())],
                },
            ],
        };

        let result = verify_changeset_roundtrip(&scenario).unwrap();
        assert!(
            result.is_consistent(),
            "Basic insert roundtrip should be consistent"
        );
    }

    #[test]
    fn test_update_roundtrip() {
        let scenario = FuzzScenario {
            schema: vec![DdlOperation::CreateTable {
                table_name: "items".to_string(),
                columns: vec![
                    ColumnDef {
                        name: "id".to_string(),
                        col_type: ColumnType::Integer,
                        nullable: false,
                        primary_key: true,
                    },
                    ColumnDef {
                        name: "value".to_string(),
                        col_type: ColumnType::Integer,
                        nullable: true,
                        primary_key: false,
                    },
                ],
                if_not_exists: true,
            }],
            operations: vec![
                DmlOperation::Insert {
                    table_name: "items".to_string(),
                    columns: vec!["id".to_string(), "value".to_string()],
                    values: vec![SqlValue::Integer(1), SqlValue::Integer(100)],
                },
                DmlOperation::Update {
                    table_name: "items".to_string(),
                    set_column: "value".to_string(),
                    set_value: SqlValue::Integer(200),
                    where_column: "id".to_string(),
                    where_value: SqlValue::Integer(1),
                },
            ],
        };

        let result = verify_changeset_roundtrip(&scenario).unwrap();
        assert!(
            result.is_consistent(),
            "Update roundtrip should be consistent"
        );
    }

    #[test]
    fn test_delete_roundtrip() {
        let scenario = FuzzScenario {
            schema: vec![DdlOperation::CreateTable {
                table_name: "logs".to_string(),
                columns: vec![
                    ColumnDef {
                        name: "id".to_string(),
                        col_type: ColumnType::Integer,
                        nullable: false,
                        primary_key: true,
                    },
                    ColumnDef {
                        name: "message".to_string(),
                        col_type: ColumnType::Text,
                        nullable: true,
                        primary_key: false,
                    },
                ],
                if_not_exists: true,
            }],
            operations: vec![
                DmlOperation::Insert {
                    table_name: "logs".to_string(),
                    columns: vec!["id".to_string(), "message".to_string()],
                    values: vec![
                        SqlValue::Integer(1),
                        SqlValue::Text("Log entry".to_string()),
                    ],
                },
                DmlOperation::Delete {
                    table_name: "logs".to_string(),
                    where_column: "id".to_string(),
                    where_value: SqlValue::Integer(1),
                },
            ],
        };

        let result = verify_changeset_roundtrip(&scenario).unwrap();
        assert!(
            result.is_consistent(),
            "Delete roundtrip should be consistent"
        );
    }

    #[test]
    fn test_null_values_roundtrip() {
        let scenario = FuzzScenario {
            schema: vec![DdlOperation::CreateTable {
                table_name: "nullable".to_string(),
                columns: vec![
                    ColumnDef {
                        name: "id".to_string(),
                        col_type: ColumnType::Integer,
                        nullable: false,
                        primary_key: true,
                    },
                    ColumnDef {
                        name: "maybe_int".to_string(),
                        col_type: ColumnType::Integer,
                        nullable: true,
                        primary_key: false,
                    },
                    ColumnDef {
                        name: "maybe_text".to_string(),
                        col_type: ColumnType::Text,
                        nullable: true,
                        primary_key: false,
                    },
                ],
                if_not_exists: true,
            }],
            operations: vec![
                DmlOperation::Insert {
                    table_name: "nullable".to_string(),
                    columns: vec![
                        "id".to_string(),
                        "maybe_int".to_string(),
                        "maybe_text".to_string(),
                    ],
                    values: vec![SqlValue::Integer(1), SqlValue::Null, SqlValue::Null],
                },
                DmlOperation::Insert {
                    table_name: "nullable".to_string(),
                    columns: vec![
                        "id".to_string(),
                        "maybe_int".to_string(),
                        "maybe_text".to_string(),
                    ],
                    values: vec![SqlValue::Integer(2), SqlValue::Integer(42), SqlValue::Null],
                },
            ],
        };

        let result = verify_changeset_roundtrip(&scenario).unwrap();
        assert!(
            result.is_consistent(),
            "NULL values roundtrip should be consistent"
        );
    }

    #[test]
    fn test_blob_values_roundtrip() {
        let scenario = FuzzScenario {
            schema: vec![DdlOperation::CreateTable {
                table_name: "bindata".to_string(),
                columns: vec![
                    ColumnDef {
                        name: "id".to_string(),
                        col_type: ColumnType::Integer,
                        nullable: false,
                        primary_key: true,
                    },
                    ColumnDef {
                        name: "data".to_string(),
                        col_type: ColumnType::Blob,
                        nullable: true,
                        primary_key: false,
                    },
                ],
                if_not_exists: true,
            }],
            operations: vec![
                DmlOperation::Insert {
                    table_name: "bindata".to_string(),
                    columns: vec!["id".to_string(), "data".to_string()],
                    values: vec![
                        SqlValue::Integer(1),
                        SqlValue::Blob(vec![0xDE, 0xAD, 0xBE, 0xEF]),
                    ],
                },
                DmlOperation::Insert {
                    table_name: "bindata".to_string(),
                    columns: vec!["id".to_string(), "data".to_string()],
                    values: vec![SqlValue::Integer(2), SqlValue::Blob(vec![])],
                },
            ],
        };

        let result = verify_changeset_roundtrip(&scenario).unwrap();
        assert!(
            result.is_consistent(),
            "BLOB values roundtrip should be consistent"
        );
    }

    #[test]
    fn test_empty_string_roundtrip() {
        let scenario = FuzzScenario {
            schema: vec![DdlOperation::CreateTable {
                table_name: "strings".to_string(),
                columns: vec![
                    ColumnDef {
                        name: "id".to_string(),
                        col_type: ColumnType::Integer,
                        nullable: false,
                        primary_key: true,
                    },
                    ColumnDef {
                        name: "text".to_string(),
                        col_type: ColumnType::Text,
                        nullable: true,
                        primary_key: false,
                    },
                ],
                if_not_exists: true,
            }],
            operations: vec![DmlOperation::Insert {
                table_name: "strings".to_string(),
                columns: vec!["id".to_string(), "text".to_string()],
                values: vec![SqlValue::Integer(1), SqlValue::Text(String::new())],
            }],
        };

        let result = verify_changeset_roundtrip(&scenario).unwrap();
        assert!(
            result.is_consistent(),
            "Empty string roundtrip should be consistent"
        );
    }

    #[test]
    fn test_special_characters_roundtrip() {
        let scenario = FuzzScenario {
            schema: vec![DdlOperation::CreateTable {
                table_name: "special".to_string(),
                columns: vec![
                    ColumnDef {
                        name: "id".to_string(),
                        col_type: ColumnType::Integer,
                        nullable: false,
                        primary_key: true,
                    },
                    ColumnDef {
                        name: "text".to_string(),
                        col_type: ColumnType::Text,
                        nullable: true,
                        primary_key: false,
                    },
                ],
                if_not_exists: true,
            }],
            operations: vec![
                DmlOperation::Insert {
                    table_name: "special".to_string(),
                    columns: vec!["id".to_string(), "text".to_string()],
                    values: vec![
                        SqlValue::Integer(1),
                        SqlValue::Text("Hello's World".to_string()),
                    ],
                },
                DmlOperation::Insert {
                    table_name: "special".to_string(),
                    columns: vec!["id".to_string(), "text".to_string()],
                    values: vec![
                        SqlValue::Integer(2),
                        SqlValue::Text("Line1\nLine2\tTabbed".to_string()),
                    ],
                },
            ],
        };

        let result = verify_changeset_roundtrip(&scenario).unwrap();
        assert!(
            result.is_consistent(),
            "Special characters roundtrip should be consistent"
        );
    }

    #[test]
    fn test_real_values_roundtrip() {
        let scenario = FuzzScenario {
            schema: vec![DdlOperation::CreateTable {
                table_name: "reals".to_string(),
                columns: vec![
                    ColumnDef {
                        name: "id".to_string(),
                        col_type: ColumnType::Integer,
                        nullable: false,
                        primary_key: true,
                    },
                    ColumnDef {
                        name: "value".to_string(),
                        col_type: ColumnType::Real,
                        nullable: true,
                        primary_key: false,
                    },
                ],
                if_not_exists: true,
            }],
            operations: vec![
                DmlOperation::Insert {
                    table_name: "reals".to_string(),
                    columns: vec!["id".to_string(), "value".to_string()],
                    values: vec![SqlValue::Integer(1), SqlValue::Real(1.23456)],
                },
                DmlOperation::Insert {
                    table_name: "reals".to_string(),
                    columns: vec!["id".to_string(), "value".to_string()],
                    values: vec![SqlValue::Integer(2), SqlValue::Real(-0.0)],
                },
                DmlOperation::Insert {
                    table_name: "reals".to_string(),
                    columns: vec!["id".to_string(), "value".to_string()],
                    values: vec![SqlValue::Integer(3), SqlValue::Real(1e10)],
                },
            ],
        };

        let result = verify_changeset_roundtrip(&scenario).unwrap();
        assert!(
            result.is_consistent(),
            "REAL values roundtrip should be consistent"
        );
    }

    #[test]
    fn test_large_integer_roundtrip() {
        let scenario = FuzzScenario {
            schema: vec![DdlOperation::CreateTable {
                table_name: "bigints".to_string(),
                columns: vec![
                    ColumnDef {
                        name: "id".to_string(),
                        col_type: ColumnType::Integer,
                        nullable: false,
                        primary_key: true,
                    },
                    ColumnDef {
                        name: "big".to_string(),
                        col_type: ColumnType::Integer,
                        nullable: true,
                        primary_key: false,
                    },
                ],
                if_not_exists: true,
            }],
            operations: vec![
                DmlOperation::Insert {
                    table_name: "bigints".to_string(),
                    columns: vec!["id".to_string(), "big".to_string()],
                    values: vec![SqlValue::Integer(1), SqlValue::Integer(i64::MAX)],
                },
                DmlOperation::Insert {
                    table_name: "bigints".to_string(),
                    columns: vec!["id".to_string(), "big".to_string()],
                    values: vec![SqlValue::Integer(2), SqlValue::Integer(i64::MIN)],
                },
            ],
        };

        let result = verify_changeset_roundtrip(&scenario).unwrap();
        assert!(
            result.is_consistent(),
            "Large integer roundtrip should be consistent"
        );
    }
}
