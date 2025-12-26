use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: [u8; 32],
    pub message_type: String,
    #[serde(with = "hex_bytes")]
    pub calldata: Vec<u8>,
    pub metadata: serde_json::Value,
    pub metadata_hash: [u8; 32],
    pub nonce: u64,
    pub timestamp: u64,
    pub domain: [u8; 32],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<u128>,
}

impl Message {
    pub fn compute_id(&self) -> [u8; 32] {
        compute_message_id(
            &self.message_type,
            &self.calldata,
            &self.metadata_hash,
            self.nonce,
            self.timestamp,
            &self.domain,
        )
    }

    pub fn verify_id(&self) -> bool {
        self.id == self.compute_id()
    }
}

pub fn compute_message_id(
    message_type: &str,
    calldata: &[u8],
    metadata_hash: &[u8; 32],
    nonce: u64,
    timestamp: u64,
    domain: &[u8; 32],
) -> [u8; 32] {
    let message_type_hash = Keccak256::digest(message_type.as_bytes());
    let calldata_hash = Keccak256::digest(calldata);

    let mut encoded = Vec::with_capacity(192);
    encoded.extend_from_slice(&message_type_hash);
    encoded.extend_from_slice(&calldata_hash);
    encoded.extend_from_slice(metadata_hash);
    encoded.extend_from_slice(&[0u8; 24]); // padding for uint64
    encoded.extend_from_slice(&nonce.to_be_bytes());
    encoded.extend_from_slice(&[0u8; 24]); // padding for uint64
    encoded.extend_from_slice(&timestamp.to_be_bytes());
    encoded.extend_from_slice(domain);

    Keccak256::digest(&encoded).into()
}

pub fn compute_metadata_hash(metadata: &serde_json::Value) -> anyhow::Result<[u8; 32]> {
    let canonical = canonicalize_json(metadata)?;
    Ok(Keccak256::digest(canonical.as_bytes()).into())
}

fn canonicalize_json(value: &serde_json::Value) -> anyhow::Result<String> {
    match value {
        serde_json::Value::Object(map) => {
            let mut pairs: Vec<_> = map.iter().collect();
            pairs.sort_by(|a, b| a.0.cmp(b.0));

            let inner: Vec<String> = pairs
                .into_iter()
                .map(|(k, v)| {
                    let canonical_v = canonicalize_json(v)?;
                    Ok(format!("\"{}\":{}", escape_json_string(k), canonical_v))
                })
                .collect::<anyhow::Result<Vec<_>>>()?;

            Ok(format!("{{{}}}", inner.join(",")))
        }
        serde_json::Value::Array(arr) => {
            let inner: Vec<String> = arr
                .iter()
                .map(canonicalize_json)
                .collect::<anyhow::Result<Vec<_>>>()?;
            Ok(format!("[{}]", inner.join(",")))
        }
        serde_json::Value::String(s) => Ok(format!("\"{}\"", escape_json_string(s))),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Bool(b) => Ok(if *b { "true" } else { "false" }.to_string()),
        serde_json::Value::Null => Ok("null".to_string()),
    }
}

fn escape_json_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => result.push(c),
        }
    }
    result
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRequest {
    pub message_type: String,
    #[serde(with = "hex_bytes")]
    pub calldata: Vec<u8>,
    pub metadata: serde_json::Value,
    pub nonce: u64,
    pub timestamp: u64,
    #[serde(with = "hex_bytes_32")]
    pub domain: [u8; 32],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageResponse {
    pub status: MessageStatus,
    #[serde(with = "hex_bytes_32")]
    pub message_id: [u8; 32],
    #[serde(default, skip_serializing_if = "Option::is_none", with = "hex_bytes_opt")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageStatus {
    Accepted,
    Rejected,
}

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

mod hex_bytes_opt {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match bytes {
            Some(b) => serializer.serialize_str(&format!("0x{}", hex::encode(b))),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Option<String> = Option::deserialize(deserializer)?;
        match s {
            Some(s) => {
                let s = s.strip_prefix("0x").unwrap_or(&s);
                Ok(Some(hex::decode(s).map_err(serde::de::Error::custom)?))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_message_id() {
        let message_type = "setValue(uint256)";
        let calldata = hex::decode("60fe47b1000000000000000000000000000000000000000000000000000000000000002a").unwrap();
        let metadata_hash = [0u8; 32];
        let nonce = 1u64;
        let timestamp = 1234567890u64;
        let domain = [0u8; 32];

        let id = compute_message_id(message_type, &calldata, &metadata_hash, nonce, timestamp, &domain);

        assert_eq!(id.len(), 32);
        assert_ne!(id, [0u8; 32]);
    }

    #[test]
    fn test_canonicalize_json() {
        let json = serde_json::json!({
            "b": 2,
            "a": 1,
            "c": {"z": 26, "y": 25}
        });

        let canonical = canonicalize_json(&json).unwrap();
        assert_eq!(canonical, r#"{"a":1,"b":2,"c":{"y":25,"z":26}}"#);
    }

    #[test]
    fn test_message_verify_id() {
        let message_type = "setValue(uint256)".to_string();
        let calldata = vec![0u8; 32];
        let metadata_hash = [0u8; 32];
        let nonce = 1u64;
        let timestamp = 1234567890u64;
        let domain = [0u8; 32];

        let id = compute_message_id(&message_type, &calldata, &metadata_hash, nonce, timestamp, &domain);

        let message = Message {
            id,
            message_type,
            calldata,
            metadata: serde_json::Value::Null,
            metadata_hash,
            nonce,
            timestamp,
            domain,
            value: None,
        };

        assert!(message.verify_id());
    }
}
