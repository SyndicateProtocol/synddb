//! Proptest generators for SQL operations
//!
//! This module provides strategies for generating random but valid SQL operations
//! that can be used in property-based testing.

use crate::operations::*;
use proptest::prelude::*;

/// Strategy for generating valid table names
pub fn table_name_strategy() -> impl Strategy<Value = String> {
    proptest::string::string_regex("tbl_[a-z]{1,8}")
        .unwrap()
        .prop_map(|s| s)
}

/// Strategy for generating valid column names
pub fn column_name_strategy() -> impl Strategy<Value = String> {
    proptest::string::string_regex("col_[a-z]{1,8}")
        .unwrap()
        .prop_map(|s| s)
}

/// Strategy for generating valid index names
pub fn index_name_strategy() -> impl Strategy<Value = String> {
    proptest::string::string_regex("idx_[a-z]{1,8}")
        .unwrap()
        .prop_map(|s| s)
}

/// Strategy for generating SQL values
pub fn sql_value_strategy() -> impl Strategy<Value = SqlValue> {
    prop_oneof![
        5 => Just(SqlValue::Null),
        20 => any::<i64>().prop_map(SqlValue::Integer),
        10 => any::<f64>()
            .prop_filter("finite", |f| f.is_finite())
            .prop_map(SqlValue::Real),
        40 => proptest::string::string_regex("[a-zA-Z0-9 ]{0,50}")
            .unwrap()
            .prop_map(SqlValue::Text),
        15 => proptest::collection::vec(any::<u8>(), 0..50)
            .prop_map(SqlValue::Blob),
        // Edge cases
        5 => Just(SqlValue::Text(String::new())), // Empty string
        5 => Just(SqlValue::Blob(Vec::new())), // Empty blob
    ]
}

/// Strategy for generating column definitions
pub fn column_def_strategy() -> impl Strategy<Value = ColumnDef> {
    (
        column_name_strategy(),
        any::<ColumnType>(),
        any::<bool>(), // nullable
        any::<bool>(), // primary_key
    )
        .prop_map(|(name, col_type, nullable, primary_key)| ColumnDef {
            name,
            col_type,
            nullable,
            primary_key,
        })
}

/// Strategy for generating a valid CREATE TABLE operation
pub fn create_table_strategy() -> impl Strategy<Value = DdlOperation> {
    (
        table_name_strategy(),
        proptest::collection::vec(column_def_strategy(), 1..5),
        any::<bool>(),
    )
        .prop_map(|(table_name, mut columns, if_not_exists)| {
            // Ensure unique column names and at most one primary key
            let mut seen = std::collections::HashSet::new();
            let mut has_pk = false;

            columns.retain_mut(|col| {
                if seen.contains(&col.name) {
                    return false;
                }
                seen.insert(col.name.clone());

                if col.primary_key {
                    if has_pk {
                        col.primary_key = false;
                    } else {
                        // SQLite's changeset mechanism works best with INTEGER PRIMARY KEY
                        // Force all primary keys to be INTEGER
                        col.col_type = ColumnType::Integer;
                        has_pk = true;
                    }
                }
                true
            });

            // Ensure at least one column
            if columns.is_empty() {
                columns.push(ColumnDef {
                    name: "id".to_string(),
                    col_type: ColumnType::Integer,
                    nullable: false,
                    primary_key: true,
                });
                has_pk = true;
            }

            // SQLite's changeset mechanism requires a primary key
            // If no primary key was generated, add one at the beginning
            if !has_pk {
                // Find a unique name for the ID column
                let pk_name = if seen.contains("id") {
                    let mut i = 0;
                    loop {
                        let name = format!("pk_{}", i);
                        if !seen.contains(name.as_str()) {
                            break name;
                        }
                        i += 1;
                    }
                } else {
                    "id".to_string()
                };

                columns.insert(
                    0,
                    ColumnDef {
                        name: pk_name,
                        col_type: ColumnType::Integer,
                        nullable: false,
                        primary_key: true,
                    },
                );
            }

            DdlOperation::CreateTable {
                table_name,
                columns,
                if_not_exists,
            }
        })
}

/// Strategy for generating INSERT operations for a known table schema
///
/// This version takes column definitions to avoid generating NULL for primary keys.
pub fn insert_strategy_with_schema(
    table_name: String,
    columns: Vec<ColumnDef>,
) -> impl Strategy<Value = DmlOperation> {
    let num_cols = columns.len();
    let table_name_clone = table_name;
    let column_names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
    let pk_or_not_null: Vec<bool> = columns
        .iter()
        .map(|c| c.primary_key || !c.nullable)
        .collect();

    proptest::collection::vec(sql_value_strategy(), num_cols..=num_cols).prop_map(move |values| {
        // Replace NULL values with integers for primary key / NOT NULL columns
        let fixed_values: Vec<SqlValue> = values
            .into_iter()
            .zip(pk_or_not_null.iter())
            .enumerate()
            .map(|(i, (val, &needs_value))| {
                if needs_value && matches!(val, SqlValue::Null) {
                    // Replace NULL with a deterministic non-null value
                    SqlValue::Integer(i as i64 + 1)
                } else {
                    val
                }
            })
            .collect();

        DmlOperation::Insert {
            table_name: table_name_clone.clone(),
            columns: column_names.clone(),
            values: fixed_values,
        }
    })
}

/// Strategy for generating INSERT operations (simple version with just column names)
pub fn insert_strategy(
    table_name: String,
    columns: Vec<String>,
) -> impl Strategy<Value = DmlOperation> {
    let num_cols = columns.len();
    let table_name_clone = table_name;
    let columns_clone = columns;

    proptest::collection::vec(sql_value_strategy(), num_cols..=num_cols).prop_map(move |values| {
        DmlOperation::Insert {
            table_name: table_name_clone.clone(),
            columns: columns_clone.clone(),
            values,
        }
    })
}

/// Strategy for generating UPDATE operations
pub fn update_strategy(
    table_name: String,
    columns: Vec<String>,
) -> impl Strategy<Value = DmlOperation> {
    let table_name_clone = table_name;

    (
        proptest::sample::select(columns.clone()),
        sql_value_strategy(),
        proptest::sample::select(columns),
        sql_value_strategy(),
    )
        .prop_map(move |(set_column, set_value, where_column, where_value)| {
            DmlOperation::Update {
                table_name: table_name_clone.clone(),
                set_column,
                set_value,
                where_column,
                where_value,
            }
        })
}

/// Strategy for generating DELETE operations
pub fn delete_strategy(
    table_name: String,
    columns: Vec<String>,
) -> impl Strategy<Value = DmlOperation> {
    let table_name_clone = table_name;

    (proptest::sample::select(columns), sql_value_strategy()).prop_map(
        move |(where_column, where_value)| DmlOperation::Delete {
            table_name: table_name_clone.clone(),
            where_column,
            where_value,
        },
    )
}

/// A test scenario representing a valid sequence of operations
#[derive(Debug, Clone)]
pub struct FuzzScenario {
    /// The schema definition (CREATE TABLE statements)
    pub schema: Vec<DdlOperation>,
    /// The DML operations to execute
    pub operations: Vec<DmlOperation>,
}

impl FuzzScenario {
    /// Generate SQL to set up the schema
    pub fn schema_sql(&self) -> Vec<String> {
        self.schema.iter().map(|op| op.to_sql()).collect()
    }

    /// Generate SQL for all operations
    pub fn operation_sql(&self) -> Vec<String> {
        self.operations.iter().map(|op| op.to_sql()).collect()
    }
}

/// Strategy for generating complete fuzz scenarios
///
/// This generates:
/// 1. A table schema with random columns
/// 2. A sequence of valid DML operations against that schema
pub fn fuzz_scenario_strategy() -> impl Strategy<Value = FuzzScenario> {
    // First generate the schema
    create_table_strategy()
        .prop_flat_map(|create_table| {
            let table_name = match &create_table {
                DdlOperation::CreateTable { table_name, .. } => table_name.clone(),
                _ => unreachable!(),
            };

            let columns: Vec<ColumnDef> = match &create_table {
                DdlOperation::CreateTable { columns, .. } => columns.clone(),
                _ => unreachable!(),
            };

            let column_names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();

            // Generate DML operations for this schema
            let ops_strategy = proptest::collection::vec(
                prop_oneof![
                    60 => insert_strategy_with_schema(table_name.clone(), columns)
                        .prop_map(Some),
                    20 => update_strategy(table_name.clone(), column_names.clone())
                        .prop_map(Some),
                    20 => delete_strategy(table_name, column_names)
                        .prop_map(Some),
                ],
                0..20,
            )
            .prop_map(|ops: Vec<Option<DmlOperation>>| ops.into_iter().flatten().collect());

            (Just(create_table), ops_strategy)
        })
        .prop_map(|(create_table, operations)| FuzzScenario {
            schema: vec![create_table],
            operations,
        })
}

/// Strategy for generating scenarios with multiple tables
pub fn multi_table_scenario_strategy() -> impl Strategy<Value = FuzzScenario> {
    // Generate 1-3 tables
    proptest::collection::vec(create_table_strategy(), 1..4)
        .prop_flat_map(|tables| {
            // Ensure unique table names
            let mut unique_tables = Vec::new();
            let mut seen = std::collections::HashSet::new();

            for table in tables {
                let name = match &table {
                    DdlOperation::CreateTable { table_name, .. } => table_name.clone(),
                    _ => unreachable!(),
                };
                if !seen.contains(&name) {
                    seen.insert(name);
                    unique_tables.push(table);
                }
            }

            if unique_tables.is_empty() {
                unique_tables.push(DdlOperation::CreateTable {
                    table_name: "tbl_default".to_string(),
                    columns: vec![ColumnDef {
                        name: "id".to_string(),
                        col_type: ColumnType::Integer,
                        nullable: false,
                        primary_key: true,
                    }],
                    if_not_exists: true,
                });
            }

            // Collect table info for DML generation (with full column definitions)
            let table_info: Vec<(String, Vec<ColumnDef>, Vec<String>)> = unique_tables
                .iter()
                .map(|t| match t {
                    DdlOperation::CreateTable {
                        table_name,
                        columns,
                        ..
                    } => (
                        table_name.clone(),
                        columns.clone(),
                        columns.iter().map(|c| c.name.clone()).collect(),
                    ),
                    _ => unreachable!(),
                })
                .collect();

            // Generate operations for random tables
            let ops_strategy = if table_info.is_empty() {
                Just(Vec::new()).boxed()
            } else {
                proptest::collection::vec(
                    proptest::sample::select(table_info).prop_flat_map(
                        |(table_name, columns, column_names)| {
                            prop_oneof![
                                60 => insert_strategy_with_schema(table_name.clone(), columns),
                                20 => update_strategy(table_name.clone(), column_names.clone()),
                                20 => delete_strategy(table_name, column_names),
                            ]
                        },
                    ),
                    0..30,
                )
                .boxed()
            };

            (Just(unique_tables), ops_strategy)
        })
        .prop_map(|(schema, operations)| FuzzScenario { schema, operations })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::{strategy::ValueTree, test_runner::TestRunner};

    #[test]
    fn test_table_name_strategy_generates_valid_names() {
        let mut runner = TestRunner::default();
        for _ in 0..100 {
            let name = table_name_strategy()
                .new_tree(&mut runner)
                .unwrap()
                .current();
            assert!(name.starts_with("tbl_"));
            assert!(name.len() <= 12);
        }
    }

    #[test]
    fn test_fuzz_scenario_generates_valid_sql() {
        let mut runner = TestRunner::default();
        for _ in 0..10 {
            let scenario = fuzz_scenario_strategy()
                .new_tree(&mut runner)
                .unwrap()
                .current();

            // Verify schema SQL is valid
            for sql in scenario.schema_sql() {
                assert!(sql.starts_with("CREATE TABLE"));
            }

            // Verify operation SQL is valid
            for sql in scenario.operation_sql() {
                assert!(
                    sql.starts_with("INSERT")
                        || sql.starts_with("UPDATE")
                        || sql.starts_with("DELETE")
                );
            }
        }
    }
}
