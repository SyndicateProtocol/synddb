//! `COSE_Sign1` helper functions for building and verifying signed messages

use super::{
    error::CborError,
    message::{CborMessageType, ParsedCoseMessage},
    verify::{
        signature_from_bytes, verify_secp256k1, verifying_key_from_bytes, verifying_key_to_bytes,
    },
};
use coset::{cbor::Value, iana, CborSerializable, CoseSign1, CoseSign1Builder, Header, Label};
use k256::ecdsa::{Signature, VerifyingKey};

/// Custom header label for sequence number (private use range)
pub const HEADER_SEQUENCE: i64 = -65537;
/// Custom header label for timestamp (private use range)
pub const HEADER_TIMESTAMP: i64 = -65538;
/// Custom header label for message type (private use range)
pub const HEADER_MSG_TYPE: i64 = -65539;

/// Build a `COSE_Sign1` structure with `SyndDB` custom headers
///
/// The structure uses:
/// - Protected header: algorithm (ES256K), sequence, timestamp, message type
/// - Unprotected header: signer public key (64 bytes, uncompressed without 0x04 prefix)
/// - Payload: the compressed message data
/// - Signature: 64-byte secp256k1 signature (r || s format)
pub(super) fn build_cose_sign1<F>(
    sequence: u64,
    timestamp: u64,
    message_type: CborMessageType,
    payload: Vec<u8>,
    signer_pubkey: &VerifyingKey,
    sign_fn: F,
) -> Result<Vec<u8>, CborError>
where
    F: FnOnce(&[u8]) -> Result<Signature, CborError>,
{
    // Convert VerifyingKey to 64-byte representation for COSE header
    let pubkey_bytes = verifying_key_to_bytes(signer_pubkey);

    // Build protected header with custom fields
    let mut protected = Header {
        alg: Some(coset::Algorithm::Assigned(iana::Algorithm::ES256K)),
        ..Default::default()
    };
    protected
        .rest
        .push((Label::Int(HEADER_SEQUENCE), Value::Integer(sequence.into())));
    protected.rest.push((
        Label::Int(HEADER_TIMESTAMP),
        Value::Integer(timestamp.into()),
    ));
    protected.rest.push((
        Label::Int(HEADER_MSG_TYPE),
        Value::Integer((message_type.as_u8() as i64).into()),
    ));

    // Build unprotected header with signer public key (64 bytes)
    let mut unprotected = Header::default();
    unprotected.rest.push((
        Label::Text("pubkey".to_string()),
        Value::Bytes(pubkey_bytes.to_vec()),
    ));

    // Create the COSE_Sign1 structure without signature first to get the Sig_structure
    let cose = CoseSign1Builder::new()
        .protected(protected.clone())
        .unprotected(unprotected.clone())
        .payload(payload.clone())
        .build();

    // Compute the Sig_structure that needs to be signed
    let tbs = cose.tbs_data(&[]);

    // Sign it
    let signature = sign_fn(&tbs)?;

    // Rebuild with signature (convert Signature to bytes for COSE storage)
    let cose_signed = CoseSign1Builder::new()
        .protected(protected)
        .unprotected(unprotected)
        .payload(payload)
        .signature(signature.to_bytes().to_vec())
        .build();

    // Serialize to CBOR
    cose_signed
        .to_vec()
        .map_err(|e| CborError::Cose(e.to_string()))
}

/// Parse a `COSE_Sign1` structure and extract `SyndDB` fields (without signature verification)
pub fn parse_cose_sign1(bytes: &[u8]) -> Result<ParsedCoseMessage, CborError> {
    let cose = CoseSign1::from_slice(bytes)?;

    // Extract protected header fields
    let protected = &cose.protected.header;

    let sequence = extract_u64_from_header(protected, HEADER_SEQUENCE)?
        .ok_or_else(|| CborError::MissingHeader("sequence".to_string()))?;

    let timestamp = extract_u64_from_header(protected, HEADER_TIMESTAMP)?
        .ok_or_else(|| CborError::MissingHeader("timestamp".to_string()))?;

    let msg_type_u8 = extract_u64_from_header(protected, HEADER_MSG_TYPE)?
        .ok_or_else(|| CborError::MissingHeader("message_type".to_string()))?
        as u8;
    let message_type = CborMessageType::from_u8(msg_type_u8)?;

    // Extract public key from unprotected header (64 bytes)
    let pubkey = extract_pubkey_bytes(&cose.unprotected)?;

    // Extract signature (must be exactly 64 bytes)
    let signature: [u8; 64] = cose.signature.as_slice().try_into().map_err(|_| {
        CborError::Cose(format!(
            "Invalid signature length: {}",
            cose.signature.len()
        ))
    })?;

    // Extract payload
    let payload = cose.payload.unwrap_or_default();

    Ok(ParsedCoseMessage {
        sequence,
        timestamp,
        message_type,
        payload,
        signature,
        pubkey,
    })
}

/// Verify `COSE_Sign1` signature and parse contents
///
/// Verifies that the signature was created by the holder of the private key
/// corresponding to the expected public key.
pub fn verify_and_parse_cose_sign1(
    bytes: &[u8],
    expected_pubkey: &VerifyingKey,
) -> Result<ParsedCoseMessage, CborError> {
    let cose = CoseSign1::from_slice(bytes)?;

    // Extract public key and verify it matches
    let pubkey = extract_pubkey(&cose.unprotected)?;
    let expected_bytes = verifying_key_to_bytes(expected_pubkey);
    let actual_bytes = verifying_key_to_bytes(&pubkey);
    if actual_bytes != expected_bytes {
        return Err(CborError::SignatureVerification(format!(
            "Public key mismatch: expected 0x{}, got 0x{}",
            hex::encode(expected_bytes),
            hex::encode(actual_bytes)
        )));
    }

    // Compute the Sig_structure that was signed
    let tbs = cose.tbs_data(&[]);

    // Get signature bytes and parse to Signature type
    let signature_bytes: [u8; 64] = cose.signature.as_slice().try_into().map_err(|_| {
        CborError::Cose(format!(
            "Invalid signature length: {}",
            cose.signature.len()
        ))
    })?;
    let signature = signature_from_bytes(&signature_bytes)?;

    // Verify signature using the consolidated verify module
    verify_secp256k1(&tbs, &signature, expected_pubkey)?;

    // Now parse the rest
    parse_cose_sign1(bytes)
}

/// Extract sequence from `COSE_Sign1` protected header (without full parse)
pub(super) fn extract_sequence(bytes: &[u8]) -> Result<u64, CborError> {
    let cose = CoseSign1::from_slice(bytes)?;
    extract_u64_from_header(&cose.protected.header, HEADER_SEQUENCE)?
        .ok_or_else(|| CborError::MissingHeader("sequence".to_string()))
}

/// Extract the CBOR-encoded protected header from a `COSE_Sign1` structure.
///
/// This returns the raw bytes of the protected header which can be used
/// to reconstruct the `Sig_structure` for signature verification.
pub(super) fn extract_protected_header(bytes: &[u8]) -> Result<Vec<u8>, CborError> {
    let cose = CoseSign1::from_slice(bytes)?;
    // The protected field contains the serialized header bytes
    // We need to re-serialize the header to get the CBOR bytes
    cose.protected
        .header
        .to_vec()
        .map_err(|e| CborError::Cose(format!("Failed to serialize protected header: {e}")))
}

/// Extract a u64 value from a protected header by label
fn extract_u64_from_header(header: &Header, label: i64) -> Result<Option<u64>, CborError> {
    for (key, value) in &header.rest {
        if let Label::Int(l) = key {
            if *l == label {
                if let Value::Integer(i) = value {
                    let n: i128 = (*i).into();
                    return Ok(Some(n as u64));
                }
            }
        }
    }
    Ok(None)
}

/// Extract public key from unprotected header as `VerifyingKey`
fn extract_pubkey(header: &Header) -> Result<VerifyingKey, CborError> {
    let bytes = extract_pubkey_bytes(header)?;
    verifying_key_from_bytes(&bytes)
}

/// Extract public key from unprotected header (64 bytes, uncompressed without prefix)
fn extract_pubkey_bytes(header: &Header) -> Result<[u8; 64], CborError> {
    for (key, value) in &header.rest {
        if let Label::Text(s) = key {
            if s == "pubkey" {
                if let Value::Bytes(b) = value {
                    return b.as_slice().try_into().map_err(|_| {
                        CborError::Cose(format!(
                            "Invalid public key length: expected 64, got {}",
                            b.len()
                        ))
                    });
                }
            }
        }
    }
    Err(CborError::MissingHeader("pubkey".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        primitives::keccak256,
        signers::{local::PrivateKeySigner, SignerSync},
    };

    /// Test private key (well-known test key, do not use in production)
    const TEST_PRIVATE_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    fn test_signer() -> PrivateKeySigner {
        TEST_PRIVATE_KEY.parse().unwrap()
    }

    fn signer_verifying_key(signer: &PrivateKeySigner) -> VerifyingKey {
        verifying_key_from_bytes(&signer.public_key().0).unwrap()
    }

    fn sign_cose(signer: &PrivateKeySigner, data: &[u8]) -> Result<Signature, CborError> {
        let hash = keccak256(data);
        let alloy_sig = signer
            .sign_hash_sync(&hash)
            .map_err(|e| CborError::Signing(e.to_string()))?;

        // Convert 65-byte alloy sig to 64-byte ECDSA sig
        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(&alloy_sig.r().to_be_bytes::<32>());
        bytes[32..].copy_from_slice(&alloy_sig.s().to_be_bytes::<32>());
        signature_from_bytes(&bytes)
    }

    /// Helper to build a valid `COSE_Sign1` structure for testing
    fn build_test_cose(
        sequence: u64,
        timestamp: u64,
        message_type: CborMessageType,
        payload: &[u8],
    ) -> Vec<u8> {
        let signer = test_signer();
        let pubkey = signer_verifying_key(&signer);
        build_cose_sign1(
            sequence,
            timestamp,
            message_type,
            payload.to_vec(),
            &pubkey,
            |data| sign_cose(&signer, data),
        )
        .unwrap()
    }

    // =========================================================================
    // extract_u64_from_header tests
    // =========================================================================

    #[test]
    fn test_extract_u64_from_header() {
        let mut header = Header::default();
        header
            .rest
            .push((Label::Int(HEADER_SEQUENCE), Value::Integer(42.into())));

        let result = extract_u64_from_header(&header, HEADER_SEQUENCE).unwrap();
        assert_eq!(result, Some(42));

        let missing = extract_u64_from_header(&header, HEADER_TIMESTAMP).unwrap();
        assert_eq!(missing, None);
    }

    // =========================================================================
    // extract_pubkey_bytes tests
    // =========================================================================

    #[test]
    fn test_extract_pubkey_bytes_valid() {
        let pubkey = [0x42u8; 64];
        let mut header = Header::default();
        header.rest.push((
            Label::Text("pubkey".to_string()),
            Value::Bytes(pubkey.to_vec()),
        ));

        let result = extract_pubkey_bytes(&header).unwrap();
        assert_eq!(result, pubkey);
    }

    #[test]
    fn test_extract_pubkey_bytes_missing() {
        let header = Header::default();
        let result = extract_pubkey_bytes(&header);
        assert!(matches!(result, Err(CborError::MissingHeader(s)) if s == "pubkey"));
    }

    #[test]
    fn test_extract_pubkey_bytes_wrong_length() {
        let mut header = Header::default();
        header.rest.push((
            Label::Text("pubkey".to_string()),
            Value::Bytes(vec![0x42; 32]),
        ));

        let result = extract_pubkey_bytes(&header);
        assert!(
            matches!(result, Err(CborError::Cose(s)) if s.contains("Invalid public key length"))
        );
    }

    #[test]
    fn test_extract_pubkey_valid_verifying_key() {
        let signer = test_signer();
        let pubkey_bytes = signer.public_key().0;
        let mut header = Header::default();
        header.rest.push((
            Label::Text("pubkey".to_string()),
            Value::Bytes(pubkey_bytes.to_vec()),
        ));

        let result = extract_pubkey(&header).unwrap();
        // Verify we can convert back to same bytes
        assert_eq!(verifying_key_to_bytes(&result), pubkey_bytes);
    }

    // =========================================================================
    // build_cose_sign1 tests
    // =========================================================================

    #[test]
    fn test_build_cose_sign1_valid() {
        let signer = test_signer();
        let pubkey = signer_verifying_key(&signer);

        let result = build_cose_sign1(
            42,
            1700000000,
            CborMessageType::Changeset,
            b"test payload".to_vec(),
            &pubkey,
            |data| sign_cose(&signer, data),
        );

        assert!(result.is_ok());
        let cose_bytes = result.unwrap();
        assert!(!cose_bytes.is_empty());

        // Verify it's valid CBOR that can be parsed
        let parsed = CoseSign1::from_slice(&cose_bytes);
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_build_cose_sign1_signing_error() {
        let signer = test_signer();
        let pubkey = signer_verifying_key(&signer);

        let result = build_cose_sign1(
            1,
            1700000000,
            CborMessageType::Changeset,
            b"payload".to_vec(),
            &pubkey,
            |_data| Err(CborError::Signing("simulated signing failure".to_string())),
        );

        assert!(matches!(result, Err(CborError::Signing(s)) if s.contains("simulated")));
    }

    // =========================================================================
    // extract_sequence tests
    // =========================================================================

    #[test]
    fn test_extract_sequence_valid() {
        let cose_bytes = build_test_cose(42, 1700000000, CborMessageType::Changeset, b"payload");

        let sequence = extract_sequence(&cose_bytes).unwrap();
        assert_eq!(sequence, 42);
    }

    #[test]
    fn test_extract_sequence_large_value() {
        let cose_bytes =
            build_test_cose(u64::MAX, 1700000000, CborMessageType::Changeset, b"payload");

        let sequence = extract_sequence(&cose_bytes).unwrap();
        assert_eq!(sequence, u64::MAX);
    }

    #[test]
    fn test_extract_sequence_invalid_cbor() {
        let invalid_bytes = vec![0xff, 0xff, 0xff];
        let result = extract_sequence(&invalid_bytes);
        assert!(result.is_err());
    }

    // =========================================================================
    // extract_protected_header tests
    // =========================================================================

    #[test]
    fn test_extract_protected_header_valid() {
        let cose_bytes = build_test_cose(42, 1700000000, CborMessageType::Changeset, b"payload");

        let header = extract_protected_header(&cose_bytes).unwrap();
        assert!(!header.is_empty());

        // Verify it's valid CBOR
        let parsed: Result<Value, _> = ciborium::from_reader(header.as_slice());
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_extract_protected_header_invalid_cbor() {
        let invalid_bytes = vec![0xff, 0xff, 0xff];
        let result = extract_protected_header(&invalid_bytes);
        assert!(result.is_err());
    }

    // =========================================================================
    // parse_cose_sign1 tests
    // =========================================================================

    #[test]
    fn test_parse_cose_sign1_valid() {
        let cose_bytes = build_test_cose(42, 1700000000, CborMessageType::Withdrawal, b"payload");

        let parsed = parse_cose_sign1(&cose_bytes).unwrap();
        assert_eq!(parsed.sequence, 42);
        assert_eq!(parsed.timestamp, 1700000000);
        assert_eq!(parsed.message_type, CborMessageType::Withdrawal);
        assert_eq!(parsed.payload, b"payload");
        let expected_pubkey = verifying_key_to_bytes(&signer_verifying_key(&test_signer()));
        assert_eq!(parsed.pubkey, expected_pubkey);
    }

    #[test]
    fn test_parse_cose_sign1_invalid_cbor() {
        let invalid_bytes = vec![0xff, 0xff, 0xff];
        let result = parse_cose_sign1(&invalid_bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_cose_sign1_missing_sequence() {
        // Build a COSE structure manually without sequence
        let mut protected = Header {
            alg: Some(coset::Algorithm::Assigned(iana::Algorithm::ES256K)),
            ..Default::default()
        };
        // Only add timestamp, not sequence
        protected.rest.push((
            Label::Int(HEADER_TIMESTAMP),
            Value::Integer(1700000000i64.into()),
        ));
        protected
            .rest
            .push((Label::Int(HEADER_MSG_TYPE), Value::Integer(0i64.into())));

        let mut unprotected = Header::default();
        unprotected.rest.push((
            Label::Text("pubkey".to_string()),
            Value::Bytes(vec![0u8; 64]),
        ));

        let cose = CoseSign1Builder::new()
            .protected(protected)
            .unprotected(unprotected)
            .payload(b"test".to_vec())
            .signature(vec![0u8; 64])
            .build();

        let cose_bytes = cose.to_vec().unwrap();
        let result = parse_cose_sign1(&cose_bytes);
        assert!(matches!(result, Err(CborError::MissingHeader(s)) if s == "sequence"));
    }

    #[test]
    fn test_parse_cose_sign1_invalid_signature_length() {
        // Build COSE with wrong signature length
        let mut protected = Header {
            alg: Some(coset::Algorithm::Assigned(iana::Algorithm::ES256K)),
            ..Default::default()
        };
        protected
            .rest
            .push((Label::Int(HEADER_SEQUENCE), Value::Integer(1i64.into())));
        protected.rest.push((
            Label::Int(HEADER_TIMESTAMP),
            Value::Integer(1700000000i64.into()),
        ));
        protected
            .rest
            .push((Label::Int(HEADER_MSG_TYPE), Value::Integer(0i64.into())));

        let mut unprotected = Header::default();
        unprotected.rest.push((
            Label::Text("pubkey".to_string()),
            Value::Bytes(vec![0u8; 64]),
        ));

        let cose = CoseSign1Builder::new()
            .protected(protected)
            .unprotected(unprotected)
            .payload(b"test".to_vec())
            .signature(vec![0u8; 32]) // Wrong length!
            .build();

        let cose_bytes = cose.to_vec().unwrap();
        let result = parse_cose_sign1(&cose_bytes);
        assert!(
            matches!(result, Err(CborError::Cose(s)) if s.contains("Invalid signature length"))
        );
    }

    // =========================================================================
    // verify_and_parse_cose_sign1 tests
    // =========================================================================

    #[test]
    fn test_verify_and_parse_valid() {
        let signer = test_signer();
        let pubkey = signer_verifying_key(&signer);
        let cose_bytes = build_test_cose(42, 1700000000, CborMessageType::Snapshot, b"payload");

        let parsed = verify_and_parse_cose_sign1(&cose_bytes, &pubkey).unwrap();
        assert_eq!(parsed.sequence, 42);
        assert_eq!(parsed.timestamp, 1700000000);
        assert_eq!(parsed.message_type, CborMessageType::Snapshot);
    }

    #[test]
    fn test_verify_and_parse_pubkey_mismatch() {
        let cose_bytes = build_test_cose(42, 1700000000, CborMessageType::Changeset, b"payload");
        // Use a different valid key (not arbitrary bytes, which wouldn't be a valid point)
        let other_signer: PrivateKeySigner =
            "0x1111111111111111111111111111111111111111111111111111111111111111"
                .parse()
                .unwrap();
        let wrong_pubkey = signer_verifying_key(&other_signer);

        let result = verify_and_parse_cose_sign1(&cose_bytes, &wrong_pubkey);
        assert!(
            matches!(result, Err(CborError::SignatureVerification(s)) if s.contains("mismatch"))
        );
    }

    #[test]
    fn test_verify_and_parse_invalid_signature() {
        let signer = test_signer();
        let pubkey = signer_verifying_key(&signer);
        let pubkey_bytes = verifying_key_to_bytes(&pubkey);

        // Build COSE with correct pubkey but garbage signature
        let mut protected = Header {
            alg: Some(coset::Algorithm::Assigned(iana::Algorithm::ES256K)),
            ..Default::default()
        };
        protected
            .rest
            .push((Label::Int(HEADER_SEQUENCE), Value::Integer(1i64.into())));
        protected.rest.push((
            Label::Int(HEADER_TIMESTAMP),
            Value::Integer(1700000000i64.into()),
        ));
        protected
            .rest
            .push((Label::Int(HEADER_MSG_TYPE), Value::Integer(0i64.into())));

        let mut unprotected = Header::default();
        unprotected.rest.push((
            Label::Text("pubkey".to_string()),
            Value::Bytes(pubkey_bytes.to_vec()),
        ));

        let cose = CoseSign1Builder::new()
            .protected(protected)
            .unprotected(unprotected)
            .payload(b"test".to_vec())
            .signature(vec![0x42u8; 64]) // Invalid signature
            .build();

        let cose_bytes = cose.to_vec().unwrap();
        let result = verify_and_parse_cose_sign1(&cose_bytes, &pubkey);
        assert!(matches!(
            result,
            Err(CborError::SignatureVerification(s)) if s.contains("verification failed")
        ));
    }

    #[test]
    fn test_verify_and_parse_invalid_cbor() {
        let invalid_bytes = vec![0xff, 0xff, 0xff];
        let pubkey = signer_verifying_key(&test_signer());

        let result = verify_and_parse_cose_sign1(&invalid_bytes, &pubkey);
        assert!(result.is_err());
    }
}
