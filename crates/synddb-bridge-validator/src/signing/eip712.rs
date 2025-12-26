use alloy::primitives::Address;
use sha3::{Digest, Keccak256};

use crate::types::Message;

const DOMAIN_TYPEHASH: &[u8] =
    b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
const MESSAGE_TYPEHASH: &[u8] = b"Message(bytes32 messageId,string messageType,bytes calldata_,bytes32 metadataHash,uint64 nonce,uint64 timestamp,bytes32 domain)";

pub fn compute_domain_separator(chain_id: u64, bridge_address: Address) -> [u8; 32] {
    let domain_typehash = Keccak256::digest(DOMAIN_TYPEHASH);
    let name_hash = Keccak256::digest(b"SyndBridge");
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

/// Compute message ID using Solidity ABI encoding.
/// Matches: keccak256(abi.encode(messageType, keccak256(calldata_), metadataHash, nonce, timestamp, domain))
pub fn compute_message_id(
    message_type: &str,
    calldata: &[u8],
    metadata_hash: &[u8; 32],
    nonce: u64,
    timestamp: u64,
    domain: &[u8; 32],
) -> [u8; 32] {
    // Solidity abi.encode for (string, bytes32, bytes32, uint64, uint64, bytes32):
    // - Head section: 6 slots of 32 bytes each
    //   - slot 0: offset to string data (192 = 0xc0)
    //   - slot 1-5: fixed size values (padded to 32 bytes)
    // - Tail section: string length + string data (padded to 32-byte boundary)

    let calldata_hash: [u8; 32] = Keccak256::digest(calldata).into();

    // Calculate string data padding
    let string_bytes = message_type.as_bytes();
    let padded_string_len = string_bytes.len().div_ceil(32) * 32;

    let mut encoded = Vec::with_capacity(192 + 32 + padded_string_len);

    // Head section
    // Slot 0: offset to string (192 = 6*32)
    let mut offset = [0u8; 32];
    offset[31] = 192;
    encoded.extend_from_slice(&offset);

    // Slot 1: keccak256(calldata)
    encoded.extend_from_slice(&calldata_hash);

    // Slot 2: metadataHash
    encoded.extend_from_slice(metadata_hash);

    // Slot 3: nonce (uint64 padded to 32 bytes)
    let mut nonce_bytes = [0u8; 32];
    nonce_bytes[24..].copy_from_slice(&nonce.to_be_bytes());
    encoded.extend_from_slice(&nonce_bytes);

    // Slot 4: timestamp (uint64 padded to 32 bytes)
    let mut timestamp_bytes = [0u8; 32];
    timestamp_bytes[24..].copy_from_slice(&timestamp.to_be_bytes());
    encoded.extend_from_slice(&timestamp_bytes);

    // Slot 5: domain
    encoded.extend_from_slice(domain);

    // Tail section
    // String length
    let mut len_bytes = [0u8; 32];
    len_bytes[24..].copy_from_slice(&(string_bytes.len() as u64).to_be_bytes());
    encoded.extend_from_slice(&len_bytes);

    // String data (padded to 32-byte boundary)
    let mut padded_string = vec![0u8; padded_string_len];
    padded_string[..string_bytes.len()].copy_from_slice(string_bytes);
    encoded.extend_from_slice(&padded_string);

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
