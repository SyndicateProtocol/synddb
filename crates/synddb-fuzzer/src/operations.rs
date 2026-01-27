//! SQL operation types for fuzzing
//!
//! Defines the various SQL operations that can be generated and executed
//! during fuzz testing.

use proptest_derive::Arbitrary;

// Note: SqlValue doesn't use #[derive(Arbitrary)] because it needs custom
// strategies for Text and Blob variants. See generators.rs for the strategy.

/// A SQL value that can be inserted into a column
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    /// NULL value
    Null,
    /// Integer value
    Integer(i64),
    /// Floating point value
    Real(f64),
    /// Text value (limited length to avoid OOM)
    Text(String),
    /// Binary blob (limited size)
    Blob(Vec<u8>),
}

impl SqlValue {
    /// Convert to SQL literal string
    pub fn to_sql_literal(&self) -> String {
        match self {
            Self::Null => "NULL".to_string(),
            Self::Integer(i) => i.to_string(),
            Self::Real(f) => {
                if f.is_nan() || f.is_infinite() {
                    "NULL".to_string() // SQLite doesn't support NaN/Inf
                } else {
                    format!("{:.15}", f)
                }
            }
            Self::Text(s) => format!("'{}'", s.replace('\'', "''")),
            Self::Blob(b) => format!("X'{}'", hex::encode(b)),
        }
    }

    /// Bind to a rusqlite statement
    pub fn bind_to(&self, stmt: &mut rusqlite::Statement<'_>, idx: usize) -> rusqlite::Result<()> {
        match self {
            Self::Null => stmt.raw_bind_parameter(idx, rusqlite::types::Null)?,
            Self::Integer(i) => stmt.raw_bind_parameter(idx, *i)?,
            Self::Real(f) => {
                if f.is_nan() || f.is_infinite() {
                    stmt.raw_bind_parameter(idx, rusqlite::types::Null)?
                } else {
                    stmt.raw_bind_parameter(idx, *f)?
                }
            }
            Self::Text(s) => stmt.raw_bind_parameter(idx, s.as_str())?,
            Self::Blob(b) => stmt.raw_bind_parameter(idx, b.as_slice())?,
        }
        Ok(())
    }
}

/// Column type for table creation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Arbitrary)]
pub enum ColumnType {
    Integer,
    Real,
    Text,
    Blob,
    Any, // No type affinity
}

impl ColumnType {
    pub const fn to_sql(&self) -> &'static str {
        match self {
            Self::Integer => "INTEGER",
            Self::Real => "REAL",
            Self::Text => "TEXT",
            Self::Blob => "BLOB",
            Self::Any => "",
        }
    }
}

/// A column definition
#[derive(Debug, Clone, PartialEq, Eq, Arbitrary)]
pub struct ColumnDef {
    #[proptest(strategy = "proptest::string::string_regex(\"col_[a-z]{1,8}\").unwrap()")]
    pub name: String,
    pub col_type: ColumnType,
    pub nullable: bool,
    pub primary_key: bool,
}

impl ColumnDef {
    pub fn to_sql(&self) -> String {
        let mut parts = vec![self.name.clone()];

        let type_str = self.col_type.to_sql();
        if !type_str.is_empty() {
            parts.push(type_str.to_string());
        }

        if self.primary_key {
            parts.push("PRIMARY KEY".to_string());
        }

        if !self.nullable && !self.primary_key {
            parts.push("NOT NULL".to_string());
        }

        parts.join(" ")
    }
}

/// DDL operation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DdlOperation {
    /// CREATE TABLE
    CreateTable {
        table_name: String,
        columns: Vec<ColumnDef>,
        if_not_exists: bool,
    },
    /// DROP TABLE
    DropTable { table_name: String, if_exists: bool },
    /// CREATE INDEX
    CreateIndex {
        index_name: String,
        table_name: String,
        columns: Vec<String>,
        unique: bool,
    },
    /// DROP INDEX
    DropIndex { index_name: String, if_exists: bool },
}

impl DdlOperation {
    pub fn to_sql(&self) -> String {
        match self {
            Self::CreateTable {
                table_name,
                columns,
                if_not_exists,
            } => {
                let if_not_exists_str = if *if_not_exists { " IF NOT EXISTS" } else { "" };
                let cols: Vec<String> = columns.iter().map(|c| c.to_sql()).collect();
                format!(
                    "CREATE TABLE{} {} ({})",
                    if_not_exists_str,
                    table_name,
                    cols.join(", ")
                )
            }
            Self::DropTable {
                table_name,
                if_exists,
            } => {
                let if_exists_str = if *if_exists { " IF EXISTS" } else { "" };
                format!("DROP TABLE{} {}", if_exists_str, table_name)
            }
            Self::CreateIndex {
                index_name,
                table_name,
                columns,
                unique,
            } => {
                let unique_str = if *unique { " UNIQUE" } else { "" };
                format!(
                    "CREATE{} INDEX {} ON {} ({})",
                    unique_str,
                    index_name,
                    table_name,
                    columns.join(", ")
                )
            }
            Self::DropIndex {
                index_name,
                if_exists,
            } => {
                let if_exists_str = if *if_exists { " IF EXISTS" } else { "" };
                format!("DROP INDEX{} {}", if_exists_str, index_name)
            }
        }
    }
}

/// DML operation
#[derive(Debug, Clone, PartialEq)]
pub enum DmlOperation {
    /// INSERT a row
    Insert {
        table_name: String,
        columns: Vec<String>,
        values: Vec<SqlValue>,
    },
    /// UPDATE rows
    Update {
        table_name: String,
        set_column: String,
        set_value: SqlValue,
        where_column: String,
        where_value: SqlValue,
    },
    /// DELETE rows
    Delete {
        table_name: String,
        where_column: String,
        where_value: SqlValue,
    },
}

impl DmlOperation {
    pub fn to_sql(&self) -> String {
        match self {
            Self::Insert {
                table_name,
                columns,
                values,
            } => {
                let cols = columns.join(", ");
                let vals: Vec<String> = values.iter().map(|v| v.to_sql_literal()).collect();
                format!(
                    "INSERT INTO {} ({}) VALUES ({})",
                    table_name,
                    cols,
                    vals.join(", ")
                )
            }
            Self::Update {
                table_name,
                set_column,
                set_value,
                where_column,
                where_value,
            } => {
                format!(
                    "UPDATE {} SET {} = {} WHERE {} = {}",
                    table_name,
                    set_column,
                    set_value.to_sql_literal(),
                    where_column,
                    where_value.to_sql_literal()
                )
            }
            Self::Delete {
                table_name,
                where_column,
                where_value,
            } => {
                format!(
                    "DELETE FROM {} WHERE {} = {}",
                    table_name,
                    where_column,
                    where_value.to_sql_literal()
                )
            }
        }
    }
}

/// A complete SQL operation that can be executed
#[derive(Debug, Clone, PartialEq)]
pub enum SqlOperation {
    Ddl(DdlOperation),
    Dml(DmlOperation),
    BeginTransaction,
    Commit,
    Rollback,
}

impl SqlOperation {
    pub fn to_sql(&self) -> String {
        match self {
            Self::Ddl(ddl) => ddl.to_sql(),
            Self::Dml(dml) => dml.to_sql(),
            Self::BeginTransaction => "BEGIN".to_string(),
            Self::Commit => "COMMIT".to_string(),
            Self::Rollback => "ROLLBACK".to_string(),
        }
    }
}

/// A sequence of operations that represents a valid database workflow
#[derive(Debug, Clone)]
pub struct OperationSequence {
    pub operations: Vec<SqlOperation>,
}

impl OperationSequence {
    pub const fn new() -> Self {
        Self {
            operations: Vec::new(),
        }
    }

    pub fn push(&mut self, op: SqlOperation) {
        self.operations.push(op);
    }
}

impl Default for OperationSequence {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sql_value_literals() {
        assert_eq!(SqlValue::Null.to_sql_literal(), "NULL");
        assert_eq!(SqlValue::Integer(42).to_sql_literal(), "42");
        assert_eq!(
            SqlValue::Text("hello".to_string()).to_sql_literal(),
            "'hello'"
        );
        assert_eq!(
            SqlValue::Text("it's".to_string()).to_sql_literal(),
            "'it''s'"
        );
        assert_eq!(SqlValue::Blob(vec![0xDE, 0xAD]).to_sql_literal(), "X'dead'");
    }

    #[test]
    fn test_column_def_to_sql() {
        let col = ColumnDef {
            name: "id".to_string(),
            col_type: ColumnType::Integer,
            nullable: false,
            primary_key: true,
        };
        assert_eq!(col.to_sql(), "id INTEGER PRIMARY KEY");
    }

    #[test]
    fn test_create_table_sql() {
        let op = DdlOperation::CreateTable {
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
        };
        assert_eq!(
            op.to_sql(),
            "CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT)"
        );
    }
}
