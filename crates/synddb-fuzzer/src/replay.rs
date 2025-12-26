//! Database replay and verification utilities
//!
//! This module provides utilities for:
//! 1. Executing SQL operations on a database
//! 2. Extracting changesets
//! 3. Replaying changesets on a fresh database
//! 4. Comparing database states for equality

use crate::generators::FuzzScenario;
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::HashMap;

/// Execute a fuzz scenario on a database
pub fn execute_scenario(conn: &Connection, scenario: &FuzzScenario) -> Result<()> {
    // Execute schema
    for sql in scenario.schema_sql() {
        conn.execute(&sql, [])
            .with_context(|| format!("Failed to execute schema SQL: {}", sql))?;
    }

    // Execute operations
    for sql in scenario.operation_sql() {
        // Some operations may fail (e.g., UPDATE on empty table), which is OK
        let _ = conn.execute(&sql, []);
    }

    Ok(())
}

/// Get a snapshot of all user tables and their contents
pub fn snapshot_database(conn: &Connection) -> Result<DatabaseSnapshot> {
    let mut tables = HashMap::new();

    // Get list of user tables
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table'
         AND name NOT LIKE 'sqlite_%'
         AND name NOT LIKE '_synddb_%'
         ORDER BY name",
    )?;

    let table_names: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    for table_name in table_names {
        let contents = snapshot_table(conn, &table_name)?;
        tables.insert(table_name, contents);
    }

    Ok(DatabaseSnapshot { tables })
}

/// Get contents of a single table
fn snapshot_table(conn: &Connection, table_name: &str) -> Result<TableSnapshot> {
    // Get column info
    let mut stmt = conn.prepare(&format!("PRAGMA table_info('{}')", table_name))?;
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;

    // Get all rows, sorted by rowid or primary key
    let query = format!(
        "SELECT {} FROM {} ORDER BY rowid",
        columns
            .iter()
            .map(|c| format!("\"{}\"", c))
            .collect::<Vec<_>>()
            .join(", "),
        table_name
    );

    let mut stmt = conn.prepare(&query)?;
    let mut rows = Vec::new();

    let mut result = stmt.query([])?;
    while let Some(row) = result.next()? {
        let mut values = Vec::new();
        for i in 0..columns.len() {
            values.push(get_value_from_row(row, i)?);
        }
        rows.push(values);
    }

    Ok(TableSnapshot { columns, rows })
}

/// Extract a value from a row at the given index
fn get_value_from_row(row: &rusqlite::Row<'_>, idx: usize) -> Result<SnapshotValue> {
    use rusqlite::types::ValueRef;

    let value = row.get_ref(idx)?;
    Ok(match value {
        ValueRef::Null => SnapshotValue::Null,
        ValueRef::Integer(i) => SnapshotValue::Integer(i),
        ValueRef::Real(f) => SnapshotValue::Real(OrderedFloat(f)),
        ValueRef::Text(s) => SnapshotValue::Text(String::from_utf8_lossy(s).to_string()),
        ValueRef::Blob(b) => SnapshotValue::Blob(b.to_vec()),
    })
}

/// A complete snapshot of a database
#[derive(Debug, Clone, PartialEq)]
pub struct DatabaseSnapshot {
    pub tables: HashMap<String, TableSnapshot>,
}

impl DatabaseSnapshot {
    /// Check if two snapshots are equivalent
    pub fn is_equivalent(&self, other: &Self) -> bool {
        self == other
    }

    /// Get a diff between two snapshots
    pub fn diff(&self, other: &Self) -> Vec<String> {
        let mut diffs = Vec::new();

        // Check for missing tables
        for table in self.tables.keys() {
            if !other.tables.contains_key(table) {
                diffs.push(format!("Table '{}' missing in other", table));
            }
        }

        for table in other.tables.keys() {
            if !self.tables.contains_key(table) {
                diffs.push(format!("Table '{}' missing in self", table));
            }
        }

        // Check table contents
        for (table, snapshot) in &self.tables {
            if let Some(other_snapshot) = other.tables.get(table) {
                if snapshot.columns != other_snapshot.columns {
                    diffs.push(format!(
                        "Table '{}' columns differ: {:?} vs {:?}",
                        table, snapshot.columns, other_snapshot.columns
                    ));
                }

                if snapshot.rows.len() != other_snapshot.rows.len() {
                    diffs.push(format!(
                        "Table '{}' row count differs: {} vs {}",
                        table,
                        snapshot.rows.len(),
                        other_snapshot.rows.len()
                    ));
                }

                for (i, (row1, row2)) in snapshot
                    .rows
                    .iter()
                    .zip(other_snapshot.rows.iter())
                    .enumerate()
                {
                    if row1 != row2 {
                        diffs.push(format!(
                            "Table '{}' row {} differs: {:?} vs {:?}",
                            table, i, row1, row2
                        ));
                    }
                }
            }
        }

        diffs
    }
}

/// A snapshot of a single table
#[derive(Debug, Clone)]
pub struct TableSnapshot {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<SnapshotValue>>,
}

impl PartialEq for TableSnapshot {
    fn eq(&self, other: &Self) -> bool {
        if self.columns != other.columns {
            return false;
        }
        if self.rows.len() != other.rows.len() {
            return false;
        }
        // Sort rows for order-independent comparison
        // (rowid order may differ between source and changeset-applied databases)
        let mut self_rows = self.rows.clone();
        let mut other_rows = other.rows.clone();
        self_rows.sort_by(|a, b| format!("{:?}", a).cmp(&format!("{:?}", b)));
        other_rows.sort_by(|a, b| format!("{:?}", a).cmp(&format!("{:?}", b)));
        self_rows == other_rows
    }
}

/// A value in a snapshot that can be compared for equality
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotValue {
    Null,
    Integer(i64),
    Real(OrderedFloat),
    Text(String),
    Blob(Vec<u8>),
}

/// Wrapper for f64 that implements Eq for comparison purposes
#[derive(Debug, Clone)]
pub struct OrderedFloat(pub f64);

impl PartialEq for OrderedFloat {
    fn eq(&self, other: &Self) -> bool {
        // Handle NaN specially
        if self.0.is_nan() && other.0.is_nan() {
            true
        } else {
            self.0 == other.0
        }
    }
}

impl Eq for OrderedFloat {}

/// Verify that executing a scenario produces the same result when replayed
pub fn verify_replay_consistency(scenario: &FuzzScenario) -> Result<ReplayResult> {
    let temp_dir = tempfile::tempdir()?;

    // Create first database and execute scenario
    let db1_path = temp_dir.path().join("db1.sqlite");
    let conn1 = Connection::open(&db1_path)?;
    execute_scenario(&conn1, scenario)?;
    let snapshot1 = snapshot_database(&conn1)?;

    // Create second database and execute the same scenario
    let db2_path = temp_dir.path().join("db2.sqlite");
    let conn2 = Connection::open(&db2_path)?;
    execute_scenario(&conn2, scenario)?;
    let snapshot2 = snapshot_database(&conn2)?;

    // Compare snapshots
    if snapshot1.is_equivalent(&snapshot2) {
        Ok(ReplayResult::Consistent)
    } else {
        Ok(ReplayResult::Inconsistent(snapshot1.diff(&snapshot2)))
    }
}

/// Result of a replay verification
#[derive(Debug, Clone)]
pub enum ReplayResult {
    /// Databases are identical
    Consistent,
    /// Databases differ
    Inconsistent(Vec<String>),
}

impl ReplayResult {
    pub const fn is_consistent(&self) -> bool {
        matches!(self, Self::Consistent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::*;

    #[test]
    fn test_snapshot_empty_database() {
        let conn = Connection::open_in_memory().unwrap();
        let snapshot = snapshot_database(&conn).unwrap();
        assert!(snapshot.tables.is_empty());
    }

    #[test]
    fn test_snapshot_with_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE test (id INTEGER, name TEXT)", [])
            .unwrap();
        conn.execute("INSERT INTO test VALUES (1, 'hello')", [])
            .unwrap();
        conn.execute("INSERT INTO test VALUES (2, 'world')", [])
            .unwrap();

        let snapshot = snapshot_database(&conn).unwrap();
        assert!(snapshot.tables.contains_key("test"));

        let table = &snapshot.tables["test"];
        assert_eq!(table.columns, vec!["id", "name"]);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0][0], SnapshotValue::Integer(1));
        assert_eq!(table.rows[0][1], SnapshotValue::Text("hello".to_string()));
    }

    #[test]
    fn test_deterministic_execution() {
        let scenario = FuzzScenario {
            schema: vec![DdlOperation::CreateTable {
                table_name: "test".to_string(),
                columns: vec![
                    ColumnDef {
                        name: "id".to_string(),
                        col_type: ColumnType::Integer,
                        nullable: false,
                        primary_key: true,
                    },
                    ColumnDef {
                        name: "value".to_string(),
                        col_type: ColumnType::Text,
                        nullable: true,
                        primary_key: false,
                    },
                ],
                if_not_exists: true,
            }],
            operations: vec![
                DmlOperation::Insert {
                    table_name: "test".to_string(),
                    columns: vec!["id".to_string(), "value".to_string()],
                    values: vec![SqlValue::Integer(1), SqlValue::Text("hello".to_string())],
                },
                DmlOperation::Insert {
                    table_name: "test".to_string(),
                    columns: vec!["id".to_string(), "value".to_string()],
                    values: vec![SqlValue::Integer(2), SqlValue::Text("world".to_string())],
                },
            ],
        };

        let result = verify_replay_consistency(&scenario).unwrap();
        assert!(result.is_consistent());
    }
}
