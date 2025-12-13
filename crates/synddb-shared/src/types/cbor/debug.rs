//! Debug utilities for converting CBOR types to human-readable JSON

use super::{batch::CborBatch, error::CborError, message::CborSignedMessage};
use alloy::primitives::keccak256;
use serde_json::{json, Value};

/// Convert `CborBatch` to a JSON Value for human inspection
pub fn batch_to_json(batch: &CborBatch) -> Result<Value, CborError> {
    let messages: Result<Vec<Value>, CborError> =
        batch.messages.iter().map(message_to_json).collect();

    Ok(json!({
        "version": batch.version,
        "start_sequence": batch.start_sequence,
        "end_sequence": batch.end_sequence,
        "created_at": batch.created_at,
        "content_hash": format!("0x{}", hex::encode(batch.content_hash)),
        "batch_signature": format!("0x{}", hex::encode(batch.batch_signature)),
        "pubkey": format!("0x{}", hex::encode(batch.pubkey)),
        "message_count": batch.messages.len(),
        "total_message_bytes": batch.total_message_bytes(),
        "messages": messages?,
    }))
}

/// Convert `CborSignedMessage` to JSON Value
pub fn message_to_json(msg: &CborSignedMessage) -> Result<Value, CborError> {
    let parsed = msg.parse_without_verify()?;

    Ok(json!({
        "sequence": parsed.sequence,
        "timestamp": parsed.timestamp,
        "message_type": format!("{:?}", parsed.message_type),
        "payload_size_compressed": parsed.payload.len(),
        "payload_hash": format!("0x{}", hex::encode(keccak256(&parsed.payload))),
        "signature": format!("0x{}", hex::encode(parsed.signature)),
        "pubkey": format!("0x{}", hex::encode(parsed.pubkey)),
        "cose_size": msg.size(),
    }))
}

/// Decompress and decode payload to JSON (for deep inspection)
///
/// This attempts to decompress the payload with zstd and parse as CBOR,
/// then convert to JSON. Falls back to hex if parsing fails.
pub fn decode_payload_to_json(payload: &[u8]) -> Result<Value, CborError> {
    use std::io::Read;

    // Try to decompress with zstd
    let decompressed = match zstd::Decoder::new(payload) {
        Ok(mut decoder) => {
            let mut buf = Vec::new();
            decoder
                .read_to_end(&mut buf)
                .map_err(|e| CborError::Compression(e.to_string()))?;
            buf
        }
        Err(_) => {
            // Not zstd compressed, use as-is
            payload.to_vec()
        }
    };

    // Try to parse as CBOR and convert to JSON
    ciborium::from_reader::<ciborium::Value, _>(decompressed.as_slice()).map_or_else(
        |_| {
            // Fall back to hex representation
            Ok(json!({
                "raw_hex": hex::encode(&decompressed),
                "size": decompressed.len(),
            }))
        },
        |cbor_value| cbor_to_json(&cbor_value),
    )
}

/// Convert a CBOR Value to JSON Value
fn cbor_to_json(cbor: &ciborium::Value) -> Result<Value, CborError> {
    use ciborium::Value as CborValue;

    match cbor {
        CborValue::Integer(i) => {
            let n: i128 = (*i).into();
            Ok(json!(n))
        }
        CborValue::Bytes(b) => Ok(json!(format!("0x{}", hex::encode(b)))),
        CborValue::Float(f) => Ok(json!(f)),
        CborValue::Text(s) => Ok(json!(s)),
        CborValue::Bool(b) => Ok(json!(b)),
        CborValue::Null => Ok(Value::Null),
        CborValue::Array(arr) => {
            let items: Result<Vec<Value>, CborError> = arr.iter().map(cbor_to_json).collect();
            Ok(Value::Array(items?))
        }
        CborValue::Map(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                let key = match k {
                    CborValue::Text(s) => s.clone(),
                    CborValue::Integer(i) => {
                        let n: i128 = (*i).into();
                        n.to_string()
                    }
                    _ => format!("{:?}", k),
                };
                obj.insert(key, cbor_to_json(v)?);
            }
            Ok(Value::Object(obj))
        }
        CborValue::Tag(tag, value) => Ok(json!({
            "_tag": tag,
            "_value": cbor_to_json(value)?,
        })),
        _ => Ok(json!(format!("{:?}", cbor))),
    }
}

impl CborBatch {
    /// Convert to human-readable JSON structure
    pub fn to_json_value(&self) -> Result<Value, CborError> {
        batch_to_json(self)
    }

    /// Pretty-print entire batch as JSON string
    pub fn to_json_pretty(&self) -> Result<String, CborError> {
        let value = self.to_json_value()?;
        serde_json::to_string_pretty(&value)
            .map_err(|e| CborError::Cbor(format!("JSON serialization failed: {}", e)))
    }
}

impl CborSignedMessage {
    /// Convert to human-readable JSON structure
    pub fn to_json_value(&self) -> Result<Value, CborError> {
        message_to_json(self)
    }

    /// Pretty-print as JSON string
    pub fn to_json_pretty(&self) -> Result<String, CborError> {
        let value = self.to_json_value()?;
        serde_json::to_string_pretty(&value)
            .map_err(|e| CborError::Cbor(format!("JSON serialization failed: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cbor_to_json_primitives() {
        use ciborium::Value as CborValue;

        assert_eq!(
            cbor_to_json(&CborValue::Integer(42.into())).unwrap(),
            json!(42)
        );
        assert_eq!(
            cbor_to_json(&CborValue::Text("hello".into())).unwrap(),
            json!("hello")
        );
        assert_eq!(cbor_to_json(&CborValue::Bool(true)).unwrap(), json!(true));
        assert_eq!(cbor_to_json(&CborValue::Null).unwrap(), Value::Null);
    }

    #[test]
    fn test_cbor_to_json_bytes() {
        use ciborium::Value as CborValue;

        let result = cbor_to_json(&CborValue::Bytes(vec![0xde, 0xad, 0xbe, 0xef])).unwrap();
        assert_eq!(result, json!("0xdeadbeef"));
    }

    #[test]
    fn test_cbor_to_json_array() {
        use ciborium::Value as CborValue;

        let arr = CborValue::Array(vec![
            CborValue::Integer(1.into()),
            CborValue::Integer(2.into()),
            CborValue::Integer(3.into()),
        ]);
        assert_eq!(cbor_to_json(&arr).unwrap(), json!([1, 2, 3]));
    }
}
