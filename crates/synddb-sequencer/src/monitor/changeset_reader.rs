//! Parse and inspect SQLite changeset format

use anyhow::Result;

pub struct ChangesetReader {
    _data: Vec<u8>,
}

impl ChangesetReader {
    pub fn new(data: Vec<u8>) -> Self {
        Self { _data: data }
    }

    /// Parse changeset to extract operations
    pub fn parse(&self) -> Result<Vec<Operation>> {
        // TODO: Parse changeset binary format
        // See https://www.sqlite.org/sessionintro.html
        Ok(vec![])
    }
}

#[derive(Debug, Clone)]
pub enum Operation {
    Insert {
        table: String,
        rowid: i64,
        values: Vec<Value>,
    },
    Update {
        table: String,
        rowid: i64,
        old_values: Vec<Value>,
        new_values: Vec<Value>,
    },
    Delete {
        table: String,
        rowid: i64,
        old_values: Vec<Value>,
    },
}

#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}
