use anyhow::Result;
use async_trait::async_trait;

use super::StorageRecord;

#[async_trait]
pub trait StoragePublisher: Send + Sync {
    async fn publish(&self, record: &StorageRecord) -> Result<String>;
    fn uri_prefix(&self) -> &str;
}
