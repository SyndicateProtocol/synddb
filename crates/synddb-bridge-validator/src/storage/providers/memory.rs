use std::{
    collections::HashMap,
    sync::{atomic::{AtomicU64, Ordering}, Mutex},
};

use anyhow::Result;
use async_trait::async_trait;

use crate::storage::{StoragePublisher, StorageRecord};

pub struct MemoryPublisher {
    records: Mutex<HashMap<String, StorageRecord>>,
    counter: AtomicU64,
}

impl std::fmt::Debug for MemoryPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryPublisher").finish_non_exhaustive()
    }
}

impl MemoryPublisher {
    pub fn new() -> Self {
        Self {
            records: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
        }
    }

    pub fn get(&self, uri: &str) -> Option<StorageRecord> {
        let key = uri.strip_prefix("memory://").unwrap_or(uri);
        self.records.lock().unwrap().get(key).cloned()
    }
}

impl Default for MemoryPublisher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StoragePublisher for MemoryPublisher {
    async fn publish(&self, record: &StorageRecord) -> Result<String> {
        let id = self.counter.fetch_add(1, Ordering::Relaxed) + 1;

        let key = format!("{}", id);
        self.records
            .lock()
            .unwrap()
            .insert(key.clone(), record.clone());

        Ok(format!("memory://{}", key))
    }

    fn uri_prefix(&self) -> &str {
        "memory://"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::record::{MessageRecord, PublicationRecord, SignatureRecord};

    #[tokio::test]
    async fn test_memory_publisher() {
        let publisher = MemoryPublisher::new();

        let record = StorageRecord {
            message: MessageRecord {
                id: [0u8; 32],
                message_type: "test()".to_string(),
                calldata: vec![],
                metadata: serde_json::Value::Null,
                metadata_hash: [0u8; 32],
                nonce: 1,
                timestamp: 1234567890,
                domain: [0u8; 32],
            },
            primary_signature: SignatureRecord {
                validator: [0u8; 20],
                signature: vec![0u8; 65],
                signed_at: 1234567890,
            },
            publication: PublicationRecord {
                published_by: [0u8; 20],
                published_at: 1234567890,
            },
        };

        let uri = publisher.publish(&record).await.unwrap();
        assert!(uri.starts_with("memory://"));

        let retrieved = publisher.get(&uri).unwrap();
        assert_eq!(retrieved.message.nonce, 1);
    }
}
