use serde::{Deserialize, Serialize};

use crate::types::Message;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageRecord {
    pub message: MessageRecord,
    pub primary_signature: SignatureRecord,
    pub publication: PublicationRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRecord {
    #[serde(with = "hex_bytes_32")]
    pub id: [u8; 32],
    pub message_type: String,
    #[serde(with = "hex_bytes")]
    pub calldata: Vec<u8>,
    pub metadata: serde_json::Value,
    #[serde(with = "hex_bytes_32")]
    pub metadata_hash: [u8; 32],
    pub nonce: u64,
    pub timestamp: u64,
    #[serde(with = "hex_bytes_32")]
    pub domain: [u8; 32],
}

impl From<&Message> for MessageRecord {
    fn from(msg: &Message) -> Self {
        Self {
            id: msg.id,
            message_type: msg.message_type.clone(),
            calldata: msg.calldata.clone(),
            metadata: msg.metadata.clone(),
            metadata_hash: msg.metadata_hash,
            nonce: msg.nonce,
            timestamp: msg.timestamp,
            domain: msg.domain,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureRecord {
    #[serde(with = "hex_bytes_20")]
    pub validator: [u8; 20],
    #[serde(with = "hex_bytes")]
    pub signature: Vec<u8>,
    pub signed_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicationRecord {
    #[serde(with = "hex_bytes_20")]
    pub published_by: [u8; 20],
    pub published_at: u64,
}

#[allow(unreachable_pub)]
mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{}", hex::encode(bytes)))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let s = s.strip_prefix("0x").unwrap_or(&s);
        hex::decode(s).map_err(serde::de::Error::custom)
    }
}

#[allow(unreachable_pub)]
mod hex_bytes_32 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{}", hex::encode(bytes)))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let s = s.strip_prefix("0x").unwrap_or(&s);
        let bytes = hex::decode(s).map_err(serde::de::Error::custom)?;
        bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 32 bytes"))
    }
}

#[allow(unreachable_pub)]
mod hex_bytes_20 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 20], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{}", hex::encode(bytes)))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 20], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let s = s.strip_prefix("0x").unwrap_or(&s);
        let bytes = hex::decode(s).map_err(serde::de::Error::custom)?;
        bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 20 bytes"))
    }
}
