//! `COSE_Sign1` helper functions for building and verifying signed messages

use super::{
    error::CborError,
    message::{CborMessageType, ParsedCoseMessage},
};
use crate::types::cbor::verify::verify_secp256k1;
use coset::{cbor::Value, iana, CborSerializable, CoseSign1, CoseSign1Builder, Header, Label};

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
/// - Signature: 64-byte secp256k1 signature (EIP-191 format)
pub(super) fn build_cose_sign1<F>(
    sequence: u64,
    timestamp: u64,
    message_type: CborMessageType,
    payload: Vec<u8>,
    signer_pubkey: [u8; 64], // TODO CLAUDE: im surprised there isn't a better data type for this
    sign_fn: F,
) -> Result<Vec<u8>, CborError>
where
    F: FnOnce(&[u8]) -> Result<[u8; 64], CborError>,
{
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
        Value::Bytes(signer_pubkey.to_vec()),
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

    // Rebuild with signature
    let cose_signed = CoseSign1Builder::new()
        .protected(protected)
        .unprotected(unprotected)
        .payload(payload)
        .signature(signature.to_vec())
        .build();

    // Serialize to CBOR
    cose_signed
        .to_vec()
        .map_err(|e| CborError::Cose(e.to_string()))
}

/// Parse a `COSE_Sign1` structure and extract `SyndDB` fields (without signature verification)
pub(super) fn parse_cose_sign1(bytes: &[u8]) -> Result<ParsedCoseMessage, CborError> {
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
    let pubkey = extract_pubkey(&cose.unprotected)?;

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
    expected_pubkey: &[u8; 64],
) -> Result<ParsedCoseMessage, CborError> {
    let cose = CoseSign1::from_slice(bytes)?;

    // Extract public key and verify it matches
    let pubkey = extract_pubkey(&cose.unprotected)?;
    if pubkey != *expected_pubkey {
        return Err(CborError::SignatureVerification(format!(
            "Public key mismatch: expected 0x{}, got 0x{}",
            hex::encode(expected_pubkey),
            hex::encode(pubkey)
        )));
    }

    // Compute the Sig_structure that was signed
    let tbs = cose.tbs_data(&[]);

    // Get signature
    let signature: [u8; 64] = cose.signature.as_slice().try_into().map_err(|_| {
        CborError::Cose(format!(
            "Invalid signature length: {}",
            cose.signature.len()
        ))
    })?;

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

/// Extract public key from unprotected header (64 bytes, uncompressed without prefix)
fn extract_pubkey(header: &Header) -> Result<[u8; 64], CborError> {
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
}
