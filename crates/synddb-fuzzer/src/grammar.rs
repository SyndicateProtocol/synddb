//! Grammar-based SQL generation for fuzzing
//!
//! This module implements grammar-based fuzzing that generates SQL statements
//! by expanding production rules. This approach complements property-based
//! testing by systematically exploring different SQL patterns.

use crate::operations::*;
use rand::{prelude::*, Rng};

/// Configuration for grammar-based SQL generation
#[derive(Debug, Clone)]
pub struct GrammarConfig {
    /// Maximum depth of nested expressions
    pub max_depth: usize,
    /// Maximum number of columns per table
    pub max_columns: usize,
    /// Maximum number of operations in a sequence
    pub max_operations: usize,
    /// Probability of generating edge case values
    pub edge_case_probability: f64,
    /// Seed for deterministic generation (None for random)
    pub seed: Option<u64>,
}

impl Default for GrammarConfig {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_columns: 5,
            max_operations: 20,
            edge_case_probability: 0.2,
            seed: None,
        }
    }
}

/// Grammar-based SQL generator
#[allow(missing_debug_implementations)]
pub struct SqlGrammar<R: Rng> {
    config: GrammarConfig,
    rng: R,
    table_counter: u32,
    column_counter: u32,
}

impl SqlGrammar<StdRng> {
    /// Create a new generator with a specific seed
    pub fn with_seed(config: GrammarConfig, seed: u64) -> Self {
        Self {
            config,
            rng: StdRng::seed_from_u64(seed),
            table_counter: 0,
            column_counter: 0,
        }
    }
}

impl SqlGrammar<ThreadRng> {
    /// Create a new generator with random seed
    pub fn new(config: GrammarConfig) -> Self {
        Self {
            config,
            rng: rand::rng(),
            table_counter: 0,
            column_counter: 0,
        }
    }
}

impl<R: Rng> SqlGrammar<R> {
    /// Generate a unique table name
    fn gen_table_name(&mut self) -> String {
        self.table_counter += 1;
        format!("tbl_{}", self.table_counter)
    }

    /// Generate a unique column name
    fn gen_column_name(&mut self) -> String {
        self.column_counter += 1;
        format!("col_{}", self.column_counter)
    }

    /// Generate a column type
    fn gen_column_type(&mut self) -> ColumnType {
        match self.rng.random_range(0..5) {
            0 => ColumnType::Integer,
            1 => ColumnType::Real,
            2 => ColumnType::Text,
            3 => ColumnType::Blob,
            4 => ColumnType::Any,
            _ => unreachable!(),
        }
    }

    /// Generate a SQL value, optionally favoring edge cases
    fn gen_value(&mut self, favor_edge_cases: bool) -> SqlValue {
        let is_edge_case =
            favor_edge_cases && self.rng.random_bool(self.config.edge_case_probability);

        if is_edge_case {
            // Edge case values that often cause bugs
            match self.rng.random_range(0..12) {
                0 => SqlValue::Null,
                1 => SqlValue::Integer(0),
                2 => SqlValue::Integer(-1),
                3 => SqlValue::Integer(i64::MAX),
                4 => SqlValue::Integer(i64::MIN),
                5 => SqlValue::Real(0.0),
                6 => SqlValue::Real(-0.0),
                7 => SqlValue::Real(f64::MIN_POSITIVE),
                8 => SqlValue::Real(f64::MAX),
                9 => SqlValue::Text(String::new()), // Empty string
                10 => SqlValue::Blob(vec![]),       // Empty blob
                11 => SqlValue::Text(" ".to_string()), // Whitespace only
                _ => unreachable!(),
            }
        } else {
            match self.rng.random_range(0..5) {
                0 => SqlValue::Null,
                1 => SqlValue::Integer(self.rng.random()),
                2 => {
                    let val: f64 = self.rng.random_range(-1e10..1e10);
                    SqlValue::Real(val)
                }
                3 => {
                    let len = self.rng.random_range(0..50);
                    let text: String = (0..len)
                        .map(|_| {
                            let c: char = self.rng.random_range(b'a'..=b'z') as char;
                            c
                        })
                        .collect();
                    SqlValue::Text(text)
                }
                4 => {
                    let len = self.rng.random_range(0..50);
                    let blob: Vec<u8> = (0..len).map(|_| self.rng.random()).collect();
                    SqlValue::Blob(blob)
                }
                _ => unreachable!(),
            }
        }
    }

    /// Generate a non-NULL value (for primary keys)
    fn gen_non_null_value(&mut self) -> SqlValue {
        match self.rng.random_range(0..4) {
            0 => SqlValue::Integer(self.rng.random()),
            1 => {
                let val: f64 = self.rng.random_range(-1e10..1e10);
                SqlValue::Real(val)
            }
            2 => {
                let len = self.rng.random_range(1..50);
                let text: String = (0..len)
                    .map(|_| {
                        let c: char = self.rng.random_range(b'a'..=b'z') as char;
                        c
                    })
                    .collect();
                SqlValue::Text(text)
            }
            3 => {
                let len = self.rng.random_range(1..50);
                let blob: Vec<u8> = (0..len).map(|_| self.rng.random()).collect();
                SqlValue::Blob(blob)
            }
            _ => unreachable!(),
        }
    }

    /// Generate a column definition
    fn gen_column_def(&mut self, is_primary_key: bool) -> ColumnDef {
        let name = self.gen_column_name();
        // Force INTEGER for primary keys (SQLite changeset requirement)
        let col_type = if is_primary_key {
            ColumnType::Integer
        } else {
            self.gen_column_type()
        };
        let nullable = !is_primary_key && self.rng.random_bool(0.7);

        ColumnDef {
            name,
            col_type,
            nullable,
            primary_key: is_primary_key,
        }
    }

    /// Generate a CREATE TABLE operation
    pub fn gen_create_table(&mut self) -> DdlOperation {
        let table_name = self.gen_table_name();
        let num_columns = self.rng.random_range(1..=self.config.max_columns);

        let mut columns = Vec::with_capacity(num_columns);

        // First column is always the primary key
        columns.push(self.gen_column_def(true));

        // Additional columns
        for _ in 1..num_columns {
            columns.push(self.gen_column_def(false));
        }

        DdlOperation::CreateTable {
            table_name,
            columns,
            if_not_exists: self.rng.random_bool(0.5),
        }
    }

    /// Generate an INSERT operation for a given table schema
    pub fn gen_insert(&mut self, table_name: &str, columns: &[ColumnDef]) -> DmlOperation {
        let column_names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
        let values: Vec<SqlValue> = columns
            .iter()
            .map(|col| {
                if col.primary_key || !col.nullable {
                    // Generate unique integer for primary key
                    if col.primary_key {
                        SqlValue::Integer(self.rng.random())
                    } else {
                        self.gen_non_null_value()
                    }
                } else {
                    self.gen_value(true)
                }
            })
            .collect();

        DmlOperation::Insert {
            table_name: table_name.to_string(),
            columns: column_names,
            values,
        }
    }

    /// Generate an UPDATE operation for a given table schema
    pub fn gen_update(&mut self, table_name: &str, columns: &[ColumnDef]) -> DmlOperation {
        let set_col = &columns[self.rng.random_range(0..columns.len())];
        let where_col = &columns[self.rng.random_range(0..columns.len())];

        DmlOperation::Update {
            table_name: table_name.to_string(),
            set_column: set_col.name.clone(),
            set_value: self.gen_value(true),
            where_column: where_col.name.clone(),
            where_value: self.gen_value(true),
        }
    }

    /// Generate a DELETE operation for a given table schema
    pub fn gen_delete(&mut self, table_name: &str, columns: &[ColumnDef]) -> DmlOperation {
        let where_col = &columns[self.rng.random_range(0..columns.len())];

        DmlOperation::Delete {
            table_name: table_name.to_string(),
            where_column: where_col.name.clone(),
            where_value: self.gen_value(true),
        }
    }

    /// Generate a DML operation for a given table schema
    pub fn gen_dml_operation(&mut self, table_name: &str, columns: &[ColumnDef]) -> DmlOperation {
        // Weight towards INSERT operations
        match self.rng.random_range(0..10) {
            0..=5 => self.gen_insert(table_name, columns),
            6..=7 => self.gen_update(table_name, columns),
            8..=9 => self.gen_delete(table_name, columns),
            _ => unreachable!(),
        }
    }

    /// Generate a complete scenario with a single table
    pub fn gen_single_table_scenario(&mut self) -> GrammarScenario {
        let create_table = self.gen_create_table();
        let (table_name, columns) = match &create_table {
            DdlOperation::CreateTable {
                table_name,
                columns,
                ..
            } => (table_name.clone(), columns.clone()),
            _ => unreachable!(),
        };

        let num_ops = self.rng.random_range(0..=self.config.max_operations);
        let operations: Vec<DmlOperation> = (0..num_ops)
            .map(|_| self.gen_dml_operation(&table_name, &columns))
            .collect();

        GrammarScenario {
            schema: vec![create_table],
            operations,
        }
    }

    /// Generate a scenario with multiple tables
    pub fn gen_multi_table_scenario(&mut self, num_tables: usize) -> GrammarScenario {
        let mut schema = Vec::with_capacity(num_tables);
        let mut tables: Vec<(String, Vec<ColumnDef>)> = Vec::with_capacity(num_tables);

        for _ in 0..num_tables {
            let create_table = self.gen_create_table();
            if let DdlOperation::CreateTable {
                table_name,
                columns,
                ..
            } = &create_table
            {
                tables.push((table_name.clone(), columns.clone()));
            }
            schema.push(create_table);
        }

        let num_ops = self.rng.random_range(0..=self.config.max_operations);
        let operations: Vec<DmlOperation> = (0..num_ops)
            .map(|_| {
                let (table_name, columns) = &tables[self.rng.random_range(0..tables.len())];
                self.gen_dml_operation(table_name, columns)
            })
            .collect();

        GrammarScenario { schema, operations }
    }
}

/// A scenario generated by the grammar-based fuzzer
#[derive(Debug, Clone)]
pub struct GrammarScenario {
    pub schema: Vec<DdlOperation>,
    pub operations: Vec<DmlOperation>,
}

impl GrammarScenario {
    /// Convert schema to SQL
    pub fn schema_sql(&self) -> Vec<String> {
        self.schema.iter().map(|op| op.to_sql()).collect()
    }

    /// Convert operations to SQL
    pub fn operation_sql(&self) -> Vec<String> {
        self.operations.iter().map(|op| op.to_sql()).collect()
    }
}

/// Edge case patterns for testing
#[derive(Debug, Clone, Copy)]
pub enum EdgeCasePattern {
    /// Empty values (empty string, empty blob)
    EmptyValues,
    /// Boundary integers (MAX, MIN, 0, -1)
    BoundaryIntegers,
    /// Special floats (-0.0, very small, very large)
    SpecialFloats,
    /// Many NULL values
    ManyNulls,
    /// Unicode strings
    UnicodeStrings,
    /// Large blobs
    LargeBlobs,
    /// Many operations on one row
    ManyUpdatesToOneRow,
    /// Insert then delete same row
    InsertThenDelete,
}

impl<R: Rng> SqlGrammar<R> {
    /// Generate a scenario targeting a specific edge case pattern
    pub fn gen_edge_case_scenario(&mut self, pattern: EdgeCasePattern) -> GrammarScenario {
        match pattern {
            EdgeCasePattern::EmptyValues => self.gen_empty_values_scenario(),
            EdgeCasePattern::BoundaryIntegers => self.gen_boundary_integers_scenario(),
            EdgeCasePattern::SpecialFloats => self.gen_special_floats_scenario(),
            EdgeCasePattern::ManyNulls => self.gen_many_nulls_scenario(),
            EdgeCasePattern::UnicodeStrings => self.gen_unicode_strings_scenario(),
            EdgeCasePattern::LargeBlobs => self.gen_large_blobs_scenario(),
            EdgeCasePattern::ManyUpdatesToOneRow => self.gen_many_updates_scenario(),
            EdgeCasePattern::InsertThenDelete => self.gen_insert_delete_scenario(),
        }
    }

    fn gen_empty_values_scenario(&mut self) -> GrammarScenario {
        let table_name = self.gen_table_name();
        let columns = vec![
            ColumnDef {
                name: "id".to_string(),
                col_type: ColumnType::Integer,
                nullable: false,
                primary_key: true,
            },
            ColumnDef {
                name: "text_col".to_string(),
                col_type: ColumnType::Text,
                nullable: true,
                primary_key: false,
            },
            ColumnDef {
                name: "blob_col".to_string(),
                col_type: ColumnType::Blob,
                nullable: true,
                primary_key: false,
            },
        ];

        let schema = vec![DdlOperation::CreateTable {
            table_name: table_name.clone(),
            columns,
            if_not_exists: true,
        }];

        let operations = vec![
            DmlOperation::Insert {
                table_name: table_name.clone(),
                columns: vec![
                    "id".to_string(),
                    "text_col".to_string(),
                    "blob_col".to_string(),
                ],
                values: vec![
                    SqlValue::Integer(1),
                    SqlValue::Text(String::new()),
                    SqlValue::Blob(vec![]),
                ],
            },
            DmlOperation::Insert {
                table_name,
                columns: vec![
                    "id".to_string(),
                    "text_col".to_string(),
                    "blob_col".to_string(),
                ],
                values: vec![
                    SqlValue::Integer(2),
                    SqlValue::Text(" ".to_string()), // whitespace
                    SqlValue::Blob(vec![0]),
                ],
            },
        ];

        GrammarScenario { schema, operations }
    }

    fn gen_boundary_integers_scenario(&mut self) -> GrammarScenario {
        let table_name = self.gen_table_name();
        let columns = vec![
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
        ];

        let schema = vec![DdlOperation::CreateTable {
            table_name: table_name.clone(),
            columns,
            if_not_exists: true,
        }];

        let operations = vec![
            DmlOperation::Insert {
                table_name: table_name.clone(),
                columns: vec!["id".to_string(), "value".to_string()],
                values: vec![SqlValue::Integer(1), SqlValue::Integer(0)],
            },
            DmlOperation::Insert {
                table_name: table_name.clone(),
                columns: vec!["id".to_string(), "value".to_string()],
                values: vec![SqlValue::Integer(2), SqlValue::Integer(-1)],
            },
            DmlOperation::Insert {
                table_name: table_name.clone(),
                columns: vec!["id".to_string(), "value".to_string()],
                values: vec![SqlValue::Integer(3), SqlValue::Integer(i64::MAX)],
            },
            DmlOperation::Insert {
                table_name,
                columns: vec!["id".to_string(), "value".to_string()],
                values: vec![SqlValue::Integer(4), SqlValue::Integer(i64::MIN)],
            },
        ];

        GrammarScenario { schema, operations }
    }

    fn gen_special_floats_scenario(&mut self) -> GrammarScenario {
        let table_name = self.gen_table_name();
        let columns = vec![
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
        ];

        let schema = vec![DdlOperation::CreateTable {
            table_name: table_name.clone(),
            columns,
            if_not_exists: true,
        }];

        let operations = vec![
            DmlOperation::Insert {
                table_name: table_name.clone(),
                columns: vec!["id".to_string(), "value".to_string()],
                values: vec![SqlValue::Integer(1), SqlValue::Real(0.0)],
            },
            DmlOperation::Insert {
                table_name: table_name.clone(),
                columns: vec!["id".to_string(), "value".to_string()],
                values: vec![SqlValue::Integer(2), SqlValue::Real(-0.0)],
            },
            DmlOperation::Insert {
                table_name: table_name.clone(),
                columns: vec!["id".to_string(), "value".to_string()],
                values: vec![SqlValue::Integer(3), SqlValue::Real(f64::MIN_POSITIVE)],
            },
            DmlOperation::Insert {
                table_name,
                columns: vec!["id".to_string(), "value".to_string()],
                values: vec![SqlValue::Integer(4), SqlValue::Real(1e308)],
            },
        ];

        GrammarScenario { schema, operations }
    }

    fn gen_many_nulls_scenario(&mut self) -> GrammarScenario {
        let table_name = self.gen_table_name();
        let columns = vec![
            ColumnDef {
                name: "id".to_string(),
                col_type: ColumnType::Integer,
                nullable: false,
                primary_key: true,
            },
            ColumnDef {
                name: "a".to_string(),
                col_type: ColumnType::Text,
                nullable: true,
                primary_key: false,
            },
            ColumnDef {
                name: "b".to_string(),
                col_type: ColumnType::Integer,
                nullable: true,
                primary_key: false,
            },
            ColumnDef {
                name: "c".to_string(),
                col_type: ColumnType::Real,
                nullable: true,
                primary_key: false,
            },
        ];

        let schema = vec![DdlOperation::CreateTable {
            table_name: table_name.clone(),
            columns,
            if_not_exists: true,
        }];

        let operations = vec![
            DmlOperation::Insert {
                table_name: table_name.clone(),
                columns: vec![
                    "id".to_string(),
                    "a".to_string(),
                    "b".to_string(),
                    "c".to_string(),
                ],
                values: vec![
                    SqlValue::Integer(1),
                    SqlValue::Null,
                    SqlValue::Null,
                    SqlValue::Null,
                ],
            },
            DmlOperation::Insert {
                table_name,
                columns: vec![
                    "id".to_string(),
                    "a".to_string(),
                    "b".to_string(),
                    "c".to_string(),
                ],
                values: vec![
                    SqlValue::Integer(2),
                    SqlValue::Text("x".to_string()),
                    SqlValue::Null,
                    SqlValue::Null,
                ],
            },
        ];

        GrammarScenario { schema, operations }
    }

    fn gen_unicode_strings_scenario(&mut self) -> GrammarScenario {
        let table_name = self.gen_table_name();
        let columns = vec![
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
        ];

        let schema = vec![DdlOperation::CreateTable {
            table_name: table_name.clone(),
            columns,
            if_not_exists: true,
        }];

        let operations = vec![
            DmlOperation::Insert {
                table_name: table_name.clone(),
                columns: vec!["id".to_string(), "text".to_string()],
                values: vec![
                    SqlValue::Integer(1),
                    SqlValue::Text("Hello World".to_string()),
                ],
            },
            DmlOperation::Insert {
                table_name: table_name.clone(),
                columns: vec!["id".to_string(), "text".to_string()],
                values: vec![
                    SqlValue::Integer(2),
                    SqlValue::Text("Line1\nLine2\tTab".to_string()),
                ],
            },
            DmlOperation::Insert {
                table_name,
                columns: vec!["id".to_string(), "text".to_string()],
                values: vec![
                    SqlValue::Integer(3),
                    SqlValue::Text("Quote's \"test\"".to_string()),
                ],
            },
        ];

        GrammarScenario { schema, operations }
    }

    fn gen_large_blobs_scenario(&mut self) -> GrammarScenario {
        let table_name = self.gen_table_name();
        let columns = vec![
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
        ];

        let schema = vec![DdlOperation::CreateTable {
            table_name: table_name.clone(),
            columns,
            if_not_exists: true,
        }];

        // Generate some larger blobs (but not too large to avoid OOM)
        let large_blob: Vec<u8> = (0..1000).map(|i| (i % 256) as u8).collect();

        let operations = vec![
            DmlOperation::Insert {
                table_name: table_name.clone(),
                columns: vec!["id".to_string(), "data".to_string()],
                values: vec![SqlValue::Integer(1), SqlValue::Blob(large_blob)],
            },
            DmlOperation::Insert {
                table_name,
                columns: vec!["id".to_string(), "data".to_string()],
                values: vec![SqlValue::Integer(2), SqlValue::Blob(vec![0xFF; 500])],
            },
        ];

        GrammarScenario { schema, operations }
    }

    fn gen_many_updates_scenario(&mut self) -> GrammarScenario {
        let table_name = self.gen_table_name();
        let columns = vec![
            ColumnDef {
                name: "id".to_string(),
                col_type: ColumnType::Integer,
                nullable: false,
                primary_key: true,
            },
            ColumnDef {
                name: "counter".to_string(),
                col_type: ColumnType::Integer,
                nullable: false,
                primary_key: false,
            },
        ];

        let schema = vec![DdlOperation::CreateTable {
            table_name: table_name.clone(),
            columns,
            if_not_exists: true,
        }];

        let mut operations = vec![DmlOperation::Insert {
            table_name: table_name.clone(),
            columns: vec!["id".to_string(), "counter".to_string()],
            values: vec![SqlValue::Integer(1), SqlValue::Integer(0)],
        }];

        // Add many updates to the same row
        for i in 1..=10 {
            operations.push(DmlOperation::Update {
                table_name: table_name.clone(),
                set_column: "counter".to_string(),
                set_value: SqlValue::Integer(i),
                where_column: "id".to_string(),
                where_value: SqlValue::Integer(1),
            });
        }

        GrammarScenario { schema, operations }
    }

    fn gen_insert_delete_scenario(&mut self) -> GrammarScenario {
        let table_name = self.gen_table_name();
        let columns = vec![
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
        ];

        let schema = vec![DdlOperation::CreateTable {
            table_name: table_name.clone(),
            columns,
            if_not_exists: true,
        }];

        let operations = vec![
            // Insert a row
            DmlOperation::Insert {
                table_name: table_name.clone(),
                columns: vec!["id".to_string(), "value".to_string()],
                values: vec![
                    SqlValue::Integer(1),
                    SqlValue::Text("temporary".to_string()),
                ],
            },
            // Delete it
            DmlOperation::Delete {
                table_name: table_name.clone(),
                where_column: "id".to_string(),
                where_value: SqlValue::Integer(1),
            },
            // Insert again with same id
            DmlOperation::Insert {
                table_name,
                columns: vec!["id".to_string(), "value".to_string()],
                values: vec![
                    SqlValue::Integer(1),
                    SqlValue::Text("permanent".to_string()),
                ],
            },
        ];

        GrammarScenario { schema, operations }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        generators::FuzzScenario, property_tests::verify_changeset_roundtrip, replay::ReplayResult,
    };

    impl From<GrammarScenario> for FuzzScenario {
        fn from(gs: GrammarScenario) -> Self {
            Self {
                schema: gs.schema,
                operations: gs.operations,
            }
        }
    }

    #[test]
    fn test_grammar_generates_valid_sql() {
        let config = GrammarConfig::default();
        let mut grammar = SqlGrammar::with_seed(config, 42);

        let scenario = grammar.gen_single_table_scenario();

        // Verify schema SQL is valid
        for sql in scenario.schema_sql() {
            assert!(sql.starts_with("CREATE TABLE"));
        }

        // Verify operation SQL is valid
        for sql in scenario.operation_sql() {
            assert!(
                sql.starts_with("INSERT") || sql.starts_with("UPDATE") || sql.starts_with("DELETE")
            );
        }
    }

    #[test]
    fn test_deterministic_with_seed() {
        let config = GrammarConfig::default();
        let mut grammar1 = SqlGrammar::with_seed(config.clone(), 42);
        let mut grammar2 = SqlGrammar::with_seed(config, 42);

        let scenario1 = grammar1.gen_single_table_scenario();
        let scenario2 = grammar2.gen_single_table_scenario();

        assert_eq!(scenario1.schema_sql(), scenario2.schema_sql());
        assert_eq!(scenario1.operation_sql(), scenario2.operation_sql());
    }

    #[test]
    fn test_edge_case_scenarios() {
        let config = GrammarConfig::default();
        let mut grammar = SqlGrammar::with_seed(config, 42);

        // Test each edge case pattern
        let patterns = [
            EdgeCasePattern::EmptyValues,
            EdgeCasePattern::BoundaryIntegers,
            EdgeCasePattern::SpecialFloats,
            EdgeCasePattern::ManyNulls,
            EdgeCasePattern::UnicodeStrings,
            EdgeCasePattern::LargeBlobs,
            EdgeCasePattern::ManyUpdatesToOneRow,
            EdgeCasePattern::InsertThenDelete,
        ];

        for pattern in patterns {
            let scenario = grammar.gen_edge_case_scenario(pattern);
            assert!(!scenario.schema.is_empty(), "Schema should not be empty");
        }
    }

    #[test]
    fn test_multi_table_scenario() {
        let config = GrammarConfig::default();
        let mut grammar = SqlGrammar::with_seed(config, 42);

        let scenario = grammar.gen_multi_table_scenario(3);

        assert_eq!(scenario.schema.len(), 3, "Should have 3 tables");
    }

    // ========================================================================
    // Changeset Integration Tests
    // ========================================================================

    #[test]
    fn test_grammar_scenarios_changeset_roundtrip() {
        let config = GrammarConfig::default();
        let mut grammar = SqlGrammar::with_seed(config, 42);

        // Test 10 random scenarios
        for _ in 0..10 {
            let scenario = grammar.gen_single_table_scenario();
            let fuzz_scenario: FuzzScenario = scenario.into();
            let result = verify_changeset_roundtrip(&fuzz_scenario);

            // May fail due to constraint violations, but should not produce inconsistent state
            if let Ok(result) = result {
                assert!(
                    result.is_consistent(),
                    "Grammar scenario should produce consistent changeset roundtrip"
                );
            }
        }
    }

    #[test]
    fn test_edge_case_changeset_roundtrips() {
        let config = GrammarConfig::default();
        let mut grammar = SqlGrammar::with_seed(config, 42);

        let patterns = [
            EdgeCasePattern::EmptyValues,
            EdgeCasePattern::BoundaryIntegers,
            EdgeCasePattern::SpecialFloats,
            EdgeCasePattern::ManyNulls,
            EdgeCasePattern::UnicodeStrings,
            EdgeCasePattern::LargeBlobs,
            EdgeCasePattern::ManyUpdatesToOneRow,
            EdgeCasePattern::InsertThenDelete,
        ];

        for pattern in patterns {
            let scenario = grammar.gen_edge_case_scenario(pattern);
            let fuzz_scenario: FuzzScenario = scenario.into();
            let result = verify_changeset_roundtrip(&fuzz_scenario);

            match result {
                Ok(ReplayResult::Consistent) => {}
                Ok(ReplayResult::Inconsistent(diffs)) => {
                    panic!(
                        "Edge case {:?} produced inconsistent roundtrip: {:?}",
                        pattern, diffs
                    );
                }
                Err(e) => {
                    // Some errors are expected (constraint violations)
                    let err_str = e.to_string();
                    let expected_errors = [
                        "UNIQUE constraint failed",
                        "NOT NULL constraint failed",
                        "PRIMARY KEY constraint failed",
                    ];
                    assert!(
                        expected_errors.iter().any(|e| err_str.contains(e)),
                        "Edge case {:?} failed with unexpected error: {}",
                        pattern,
                        e
                    );
                }
            }
        }
    }

    #[test]
    fn test_multi_table_changeset_roundtrip() {
        let config = GrammarConfig::default();
        let mut grammar = SqlGrammar::with_seed(config, 42);

        let scenario = grammar.gen_multi_table_scenario(2);
        let fuzz_scenario: FuzzScenario = scenario.into();
        let result = verify_changeset_roundtrip(&fuzz_scenario);

        if let Ok(result) = result {
            assert!(
                result.is_consistent(),
                "Multi-table grammar scenario should produce consistent roundtrip"
            );
        }
    }
}
