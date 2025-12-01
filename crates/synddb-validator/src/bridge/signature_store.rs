//! Signature storage for relayer pickup
//!
//! Stores signed messages so that relayers can retrieve them via the HTTP API
//! and submit them to the bridge contract.

use super::MessageSignature;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, info};

/// Thread-safe storage for bridge message signatures
///
/// Signatures are stored by message ID and can be retrieved by relayers
/// via the `/signatures/*` API endpoints.
#[derive(Debug, Clone)]
pub struct SignatureStore {
    inner: Arc<SignatureStoreInner>,
}

#[derive(Debug)]
struct SignatureStoreInner {
    /// Signatures indexed by message ID
    signatures: RwLock<HashMap<String, MessageSignature>>,
    /// List of pending (not yet submitted) message IDs in order
    pending: RwLock<Vec<String>>,
}

impl SignatureStore {
    /// Create a new empty signature store
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SignatureStoreInner {
                signatures: RwLock::new(HashMap::new()),
                pending: RwLock::new(Vec::new()),
            }),
        }
    }

    /// Store a signature for a message
    ///
    /// If a signature for this message already exists, it will be replaced.
    pub fn store(&self, signature: MessageSignature) {
        let message_id = signature.message_id.clone();

        let mut sigs = self.inner.signatures.write().unwrap();
        let is_new = !sigs.contains_key(&message_id);
        sigs.insert(message_id.clone(), signature);

        if is_new {
            let mut pending = self.inner.pending.write().unwrap();
            pending.push(message_id.clone());

            info!(message_id = %message_id, "Stored new signature");
        } else {
            debug!(message_id = %message_id, "Updated existing signature");
        }
    }

    /// Get a signature by message ID
    pub fn get(&self, message_id: &str) -> Option<MessageSignature> {
        self.inner
            .signatures
            .read()
            .unwrap()
            .get(message_id)
            .cloned()
    }

    /// Get all pending message IDs
    pub fn pending_ids(&self) -> Vec<String> {
        self.inner.pending.read().unwrap().clone()
    }

    /// Get all pending signatures
    pub fn pending_signatures(&self) -> Vec<MessageSignature> {
        let pending = self.inner.pending.read().unwrap();
        let sigs = self.inner.signatures.read().unwrap();

        pending
            .iter()
            .filter_map(|id| sigs.get(id).cloned())
            .collect()
    }

    /// Mark a message as submitted (removes from pending list)
    ///
    /// The signature is kept in storage for reference but won't appear in pending list.
    pub fn mark_submitted(&self, message_id: &str) {
        let mut pending = self.inner.pending.write().unwrap();
        pending.retain(|id| id != message_id);

        debug!(message_id = %message_id, "Marked signature as submitted");
    }

    /// Remove a signature completely
    pub fn remove(&self, message_id: &str) -> Option<MessageSignature> {
        self.mark_submitted(message_id);
        self.inner.signatures.write().unwrap().remove(message_id)
    }

    /// Get the number of stored signatures
    pub fn len(&self) -> usize {
        self.inner.signatures.read().unwrap().len()
    }

    /// Check if the store is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the number of pending signatures
    pub fn pending_count(&self) -> usize {
        self.inner.pending.read().unwrap().len()
    }
}

impl Default for SignatureStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::Address;

    fn mock_signature(id: &str) -> MessageSignature {
        MessageSignature {
            message_id: id.to_string(),
            signature: vec![0u8; 65],
            signer: Address::ZERO,
            signed_at: 1700000000,
        }
    }

    #[test]
    fn test_store_and_get() {
        let store = SignatureStore::new();

        store.store(mock_signature("0x1234"));

        let retrieved = store.get("0x1234").unwrap();
        assert_eq!(retrieved.message_id, "0x1234");
    }

    #[test]
    fn test_pending_list() {
        let store = SignatureStore::new();

        store.store(mock_signature("0x1111"));
        store.store(mock_signature("0x2222"));
        store.store(mock_signature("0x3333"));

        let pending = store.pending_ids();
        assert_eq!(pending.len(), 3);
        assert_eq!(pending[0], "0x1111");
        assert_eq!(pending[1], "0x2222");
        assert_eq!(pending[2], "0x3333");
    }

    #[test]
    fn test_mark_submitted() {
        let store = SignatureStore::new();

        store.store(mock_signature("0x1111"));
        store.store(mock_signature("0x2222"));

        store.mark_submitted("0x1111");

        let pending = store.pending_ids();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0], "0x2222");

        // Signature should still be retrievable
        assert!(store.get("0x1111").is_some());
    }

    #[test]
    fn test_remove() {
        let store = SignatureStore::new();

        store.store(mock_signature("0x1111"));
        store.store(mock_signature("0x2222"));

        let removed = store.remove("0x1111");
        assert!(removed.is_some());

        assert!(store.get("0x1111").is_none());
        assert_eq!(store.pending_count(), 1);
    }

    #[test]
    fn test_update_existing() {
        let store = SignatureStore::new();

        let sig1 = mock_signature("0x1111");
        store.store(sig1);

        let mut sig2 = mock_signature("0x1111");
        sig2.signed_at = 1700000001;
        store.store(sig2);

        // Should only be one pending
        assert_eq!(store.pending_count(), 1);

        // Should have updated timestamp
        let retrieved = store.get("0x1111").unwrap();
        assert_eq!(retrieved.signed_at, 1700000001);
    }

    #[test]
    fn test_pending_signatures() {
        let store = SignatureStore::new();

        store.store(mock_signature("0x1111"));
        store.store(mock_signature("0x2222"));
        store.mark_submitted("0x1111");

        let pending = store.pending_signatures();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].message_id, "0x2222");
    }
}
