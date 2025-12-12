//! Serde helpers for binary data encoding

/// Base64 serialization/deserialization for binary data in JSON
///
/// Use with `#[serde(with = "base64_serde")]` on `Vec<u8>` fields.
pub mod base64_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        encoded.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        use base64::Engine;
        let s = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(&s)
            .map_err(serde::de::Error::custom)
    }
}

/// Base64 serialization/deserialization for optional binary data in JSON
///
/// Use with `#[serde(with = "base64_serde_opt")]` on `Option<Vec<u8>>` fields.
pub mod base64_serde_opt {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(bytes: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use base64::Engine;
        match bytes {
            Some(b) => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(b);
                encoded.serialize(serializer)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        use base64::Engine;
        let opt: Option<String> = Option::deserialize(deserializer)?;
        match opt {
            Some(s) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&s)
                    .map_err(serde::de::Error::custom)?;
                Ok(Some(bytes))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct TestStruct {
        #[serde(with = "base64_serde")]
        data: Vec<u8>,
    }

    #[test]
    fn test_base64_roundtrip() {
        let original = TestStruct {
            data: b"hello world".to_vec(),
        };

        let json = serde_json::to_string(&original).unwrap();
        assert!(json.contains("aGVsbG8gd29ybGQ=")); // "hello world" in base64

        let decoded: TestStruct = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_base64_empty() {
        let original = TestStruct { data: vec![] };
        let json = serde_json::to_string(&original).unwrap();
        let decoded: TestStruct = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
    }
}
