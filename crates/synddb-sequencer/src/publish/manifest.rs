//! Track published batches (sequence, DA location, hash)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishManifest {
    /// Map of sequence number to publish record
    records: HashMap<u64, PublishRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishRecord {
    pub sequence: u64,
    pub hash: String,
    pub locations: HashMap<String, String>, // layer -> reference
    pub timestamp: u64,
}

impl PublishManifest {
    pub fn new() -> Self {
        Self {
            records: HashMap::new(),
        }
    }

    pub fn add_record(&mut self, record: PublishRecord) {
        self.records.insert(record.sequence, record);
    }

    pub fn get_record(&self, sequence: u64) -> Option<&PublishRecord> {
        self.records.get(&sequence)
    }

    pub fn latest_sequence(&self) -> Option<u64> {
        self.records.keys().max().copied()
    }
}

impl Default for PublishManifest {
    fn default() -> Self {
        Self::new()
    }
}
