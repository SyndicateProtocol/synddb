use alloy::primitives::Address;
use sha3::{Digest, Keccak256};

use crate::types::Message;

const DOMAIN_TYPEHASH: &[u8] = b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
const MESSAGE_TYPEHASH: &[u8] = b"Message(bytes32 messageId,string messageType,bytes calldata_,bytes32 metadataHash,uint64 nonce,uint64 timestamp,bytes32 domain)";

pub fn compute_domain_separator(chain_id: u64, bridge_address: Address) -> [u8; 32] {
    let domain_typehash = Keccak256::digest(DOMAIN_TYPEHASH);
    let name_hash = Keccak256::digest(b"MessageBridge");
    let version_hash = Keccak256::digest(b"1");

    let mut chain_id_bytes = [0u8; 32];
    chain_id_bytes[24..].copy_from_slice(&chain_id.to_be_bytes());

    let mut address_bytes = [0u8; 32];
    address_bytes[12..].copy_from_slice(bridge_address.as_slice());

    let mut encoded = Vec::with_capacity(160);
    encoded.extend_from_slice(&domain_typehash);
    encoded.extend_from_slice(&name_hash);
    encoded.extend_from_slice(&version_hash);
    encoded.extend_from_slice(&chain_id_bytes);
    encoded.extend_from_slice(&address_bytes);

    Keccak256::digest(&encoded).into()
}

pub fn compute_struct_hash(message: &Message) -> [u8; 32] {
    let typehash = Keccak256::digest(MESSAGE_TYPEHASH);
    let message_type_hash = Keccak256::digest(message.message_type.as_bytes());
    let calldata_hash = Keccak256::digest(&message.calldata);

    let mut nonce_bytes = [0u8; 32];
    nonce_bytes[24..].copy_from_slice(&message.nonce.to_be_bytes());

    let mut timestamp_bytes = [0u8; 32];
    timestamp_bytes[24..].copy_from_slice(&message.timestamp.to_be_bytes());

    let mut encoded = Vec::with_capacity(256);
    encoded.extend_from_slice(&typehash);
    encoded.extend_from_slice(&message.id);
    encoded.extend_from_slice(&message_type_hash);
    encoded.extend_from_slice(&calldata_hash);
    encoded.extend_from_slice(&message.metadata_hash);
    encoded.extend_from_slice(&nonce_bytes);
    encoded.extend_from_slice(&timestamp_bytes);
    encoded.extend_from_slice(&message.domain);

    Keccak256::digest(&encoded).into()
}

pub fn compute_digest(domain_separator: &[u8; 32], struct_hash: &[u8; 32]) -> [u8; 32] {
    let mut encoded = Vec::with_capacity(66);
    encoded.extend_from_slice(b"\x19\x01");
    encoded.extend_from_slice(domain_separator);
    encoded.extend_from_slice(struct_hash);

    Keccak256::digest(&encoded).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_domain_separator() {
        let chain_id = 1u64;
        let bridge_address = Address::ZERO;

        let separator = compute_domain_separator(chain_id, bridge_address);
        assert_ne!(separator, [0u8; 32]);
    }

    #[test]
    fn test_struct_hash() {
        let message = Message {
            id: [0u8; 32],
            message_type: "setValue(uint256)".to_string(),
            calldata: vec![0u8; 32],
            metadata: serde_json::Value::Null,
            metadata_hash: [0u8; 32],
            nonce: 1,
            timestamp: 1234567890,
            domain: [0u8; 32],
            value: None,
        };

        let struct_hash = compute_struct_hash(&message);
        assert_ne!(struct_hash, [0u8; 32]);
    }

    #[test]
    fn test_compute_digest() {
        let domain_separator = [1u8; 32];
        let struct_hash = [2u8; 32];

        let digest = compute_digest(&domain_separator, &struct_hash);
        assert_ne!(digest, [0u8; 32]);
    }
}
