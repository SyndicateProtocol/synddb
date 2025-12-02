//! GCP Confidential Space attestation verification

use crate::jwt::{decode_base64url, parse_jwt, JwkKey, ParsedJwt};
use alloc::format;
use alloc::string::String;
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::signature::Verifier;
use rsa::{BigUint, RsaPublicKey};
use sha2::Sha256;

#[cfg(feature = "sp1")]
use alloy::sol;

// SP1 public values struct for on-chain verification.
// Must match the Solidity definition in the verifier contract.
#[cfg(feature = "sp1")]
sol! {
    struct PublicValuesStruct {
        // Hash of the JWKS key that signed this token (keccak256 of kid)
        bytes32 jwk_key_hash;
        // Token validity window start (iat - issued at)
        uint64 validity_window_start;
        // Token validity window end (exp - expiration)
        uint64 validity_window_end;
        // Container image digest (keccak256 of the sha256:... string)
        bytes32 image_digest_hash;
        // TEE signing key address (derived from public key in token, if any)
        address tee_signing_key;
        // Whether secure boot was enabled
        bool secboot;
        // Audience hash (keccak256 of audience string)
        bytes32 audience_hash;
    }
}

/// Verification errors
#[derive(Debug)]
pub enum VerificationError {
    /// JWT parsing failed
    JwtParseError(&'static str),
    /// Algorithm not supported (expected RS256)
    UnsupportedAlgorithm(String),
    /// Key ID not found in provided JWKS
    KeyNotFound(String),
    /// JWK is not an RSA key
    InvalidKeyType(String),
    /// Failed to decode RSA key components
    KeyDecodeError(&'static str),
    /// Failed to construct RSA public key
    RsaKeyError(String),
    /// Signature verification failed
    SignatureVerificationFailed(String),
    /// Invalid issuer (expected confidentialcomputing.googleapis.com)
    InvalidIssuer(String),
    /// Invalid software name (expected CONFIDENTIAL_SPACE)
    InvalidSwname(String),
    /// Token expired
    TokenExpired { exp: u64 },
    /// Token not yet valid
    TokenNotYetValid { nbf: u64 },
}

/// Result of successful attestation verification
#[derive(Debug)]
pub struct ValidationResult {
    /// Signing key ID from JWKS
    pub signing_key_id: String,
    /// Token issued at timestamp
    pub validity_window_start: u64,
    /// Token expiration timestamp
    pub validity_window_end: u64,
    /// Container image digest (e.g., "sha256:61bb0cf...")
    pub image_digest: String,
    /// Whether secure boot was enabled
    pub secboot: bool,
    /// Audience the token was issued for
    pub audience: String,
    /// Nonce (if provided during token request)
    pub nonce: Option<String>,
    /// Debug status
    pub dbgstat: String,
    /// Hardware model
    pub hwmodel: String,
}

/// Verify a GCP Confidential Space attestation JWT
///
/// # Arguments
/// * `jwt_bytes` - Raw JWT token bytes
/// * `jwk` - The JWK public key to verify against
/// * `expected_audience` - Optional audience to validate (if None, skips audience check)
/// * `current_time` - Optional current Unix timestamp for expiry check (if None, skips time checks)
///
/// # Returns
/// * `Ok(ValidationResult)` - Token is valid, contains extracted claims
/// * `Err(VerificationError)` - Token validation failed
pub fn verify_gcp_cs_attestation(
    jwt_bytes: &[u8],
    jwk: &JwkKey,
    expected_audience: Option<&str>,
    current_time: Option<u64>,
) -> Result<ValidationResult, VerificationError> {
    // Parse the JWT
    let parsed = parse_jwt(jwt_bytes).map_err(VerificationError::JwtParseError)?;

    // Verify algorithm
    if parsed.header.alg != "RS256" {
        return Err(VerificationError::UnsupportedAlgorithm(
            parsed.header.alg.clone(),
        ));
    }

    // Verify key ID matches
    if parsed.header.kid != jwk.kid {
        return Err(VerificationError::KeyNotFound(parsed.header.kid.clone()));
    }

    // Verify signature
    verify_rs256_signature(&parsed, jwk)?;

    // Validate issuer
    if parsed.claims.iss != "https://confidentialcomputing.googleapis.com" {
        return Err(VerificationError::InvalidIssuer(parsed.claims.iss.clone()));
    }

    // Validate software name
    if parsed.claims.swname != "CONFIDENTIAL_SPACE" {
        return Err(VerificationError::InvalidSwname(
            parsed.claims.swname.clone(),
        ));
    }

    // Validate audience if provided
    if let Some(expected_aud) = expected_audience {
        if parsed.claims.aud != expected_aud {
            return Err(VerificationError::InvalidIssuer(format!(
                "Expected audience '{}', got '{}'",
                expected_aud, parsed.claims.aud
            )));
        }
    }

    // Validate time bounds if current_time provided
    if let Some(now) = current_time {
        if now >= parsed.claims.exp {
            return Err(VerificationError::TokenExpired {
                exp: parsed.claims.exp,
            });
        }
        if now < parsed.claims.nbf {
            return Err(VerificationError::TokenNotYetValid {
                nbf: parsed.claims.nbf,
            });
        }
    }

    Ok(ValidationResult {
        signing_key_id: parsed.header.kid,
        validity_window_start: parsed.claims.iat,
        validity_window_end: parsed.claims.exp,
        image_digest: parsed.claims.submods.container.image_digest.clone(),
        secboot: parsed.claims.secboot,
        audience: parsed.claims.aud.clone(),
        nonce: parsed.claims.eat_nonce.clone(),
        dbgstat: parsed.claims.dbgstat.clone(),
        hwmodel: parsed.claims.hwmodel.clone(),
    })
}

/// Verify RS256 (RSASSA-PKCS1-v1_5 with SHA-256) signature
fn verify_rs256_signature(parsed: &ParsedJwt, jwk: &JwkKey) -> Result<(), VerificationError> {
    // Verify key type
    if jwk.kty != "RSA" {
        return Err(VerificationError::InvalidKeyType(jwk.kty.clone()));
    }

    // Decode modulus and exponent from base64url
    let n_bytes =
        decode_base64url(&jwk.n).map_err(|_| VerificationError::KeyDecodeError("modulus"))?;
    let e_bytes =
        decode_base64url(&jwk.e).map_err(|_| VerificationError::KeyDecodeError("exponent"))?;

    // Build RSA public key
    let n = BigUint::from_bytes_be(&n_bytes);
    let e = BigUint::from_bytes_be(&e_bytes);

    let public_key = RsaPublicKey::new(n, e)
        .map_err(|e| VerificationError::RsaKeyError(format!("{:?}", e)))?;

    let verifying_key = VerifyingKey::<Sha256>::new(public_key);

    // Parse signature
    let signature = Signature::try_from(parsed.signature.as_slice())
        .map_err(|e| VerificationError::SignatureVerificationFailed(format!("{:?}", e)))?;

    // Verify
    verifying_key
        .verify(&parsed.signing_input, &signature)
        .map_err(|e| VerificationError::SignatureVerificationFailed(format!("{:?}", e)))?;

    Ok(())
}

/// Find a JWK by key ID from a list of keys
pub fn find_jwk_by_kid<'a>(keys: &'a [JwkKey], kid: &str) -> Option<&'a JwkKey> {
    keys.iter().find(|k| k.kid == kid)
}

/// Extract the key ID from a JWT without full parsing/verification
/// Useful for looking up the correct JWK before verification
pub fn extract_kid_from_jwt(jwt_bytes: &[u8]) -> Result<String, &'static str> {
    let token_str = core::str::from_utf8(jwt_bytes).map_err(|_| "Invalid UTF-8 in JWT")?;

    let header_end = token_str.find('.').ok_or("Invalid JWT format")?;
    let header_b64 = &token_str[..header_end];

    let header_bytes = decode_base64url(header_b64)?;

    #[derive(serde::Deserialize)]
    struct MinimalHeader {
        kid: String,
    }

    let header: MinimalHeader =
        serde_json::from_slice(&header_bytes).map_err(|_| "Failed to parse JWT header")?;

    Ok(header.kid)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test data from the captured samples
    const SAMPLE_JWT: &[u8] = b"eyJhbGciOiJSUzI1NiIsImtpZCI6ImQ2ZDUwNzFhYjc1MjQ2YTQyYWNmYTQ2ZDI5MzE2MzExY2RhYjUxZjciLCJ0eXAiOiJKV1QifQ.eyJhdWQiOiJodHRwczovL3N5bmRkYi1zZXF1ZW5jZXIuZXhhbXBsZS5jb20iLCJleHAiOjE3NjQ3MTEzNTcsImlhdCI6MTc2NDcwNzc1NywiaXNzIjoiaHR0cHM6Ly9jb25maWRlbnRpYWxjb21wdXRpbmcuZ29vZ2xlYXBpcy5jb20iLCJuYmYiOjE3NjQ3MDc3NTcsInN1YiI6Imh0dHBzOi8vd3d3Lmdvb2dsZWFwaXMuY29tL2NvbXB1dGUvdjEvcHJvamVjdHMvc3luZC1kYi10ZXN0aW5nL3pvbmVzL3VzLWNlbnRyYWwxLWEvaW5zdGFuY2VzL2NzLWF0dGVzdGF0aW9uLXZtIiwiZWF0X3Byb2ZpbGUiOiJodHRwczovL2Nsb3VkLmdvb2dsZS5jb20vY29uZmlkZW50aWFsLWNvbXB1dGluZy9jb25maWRlbnRpYWwtc3BhY2UvZG9jcy9yZWZlcmVuY2UvdG9rZW4tY2xhaW1zIiwic2VjYm9vdCI6dHJ1ZSwib2VtaWQiOjExMTI5LCJod21vZGVsIjoiR0NQX0FNRF9TRVYiLCJzd25hbWUiOiJDT05GSURFTlRJQUxfU1BBQ0UiLCJzd3ZlcnNpb24iOlsiMjUxMDAxIl0sImRiZ3N0YXQiOiJlbmFibGVkIiwic3VibW9kcyI6eyJjb25maWRlbnRpYWxfc3BhY2UiOnsibW9uaXRvcmluZ19lbmFibGVkIjp7Im1lbW9yeSI6ZmFsc2V9fSwiY29udGFpbmVyIjp7ImltYWdlX3JlZmVyZW5jZSI6InVzLWNlbnRyYWwxLWRvY2tlci5wa2cuZGV2L3N5bmQtZGItdGVzdGluZy9zeW5kZGItdGVzdC9jcy1hdHRlc3RhdGlvbi1zYW1wbGU6bGF0ZXN0IiwiaW1hZ2VfZGlnZXN0Ijoic2hhMjU2OjYxYmIwY2YwMDc4OTE2MmU5OTQwYTk3NjIxYTZkNGU1MzEyNjYzODQyMWFlYTE4MWQ4YmZmMzlkNmUyZjIxZmEiLCJyZXN0YXJ0X3BvbGljeSI6Ik5ldmVyIiwiaW1hZ2VfaWQiOiJzaGEyNTY6ZGFhMWQ0YzE2ZjhmYjkzZWJjOTJkZWJlYWY3NjAxOWY2YzUyMTFiZWRkMDJkOTEzM2FjNmE4MmRlYjdkNDJhNSIsImVudl9vdmVycmlkZSI6eyJBVFRFU1RBVElPTl9BVURJRU5DRSI6Imh0dHBzOi8vc3luZGRiLXNlcXVlbmNlci5leGFtcGxlLmNvbSIsIk9VVFBVVF9CVUNLRVQiOiJzeW5kLWRiLXRlc3RpbmctY3MtYXR0ZXN0YXRpb24tc2FtcGxlcyJ9LCJlbnYiOnsiQVRURVNUQVRJT05fQVVESUVOQ0UiOiJodHRwczovL3N5bmRkYi1zZXF1ZW5jZXIuZXhhbXBsZS5jb20iLCJIT1NUTkFNRSI6ImNzLWF0dGVzdGF0aW9uLXZtIiwiT1VUUFVUX0JVQ0tFVCI6InN5bmQtZGItdGVzdGluZy1jcy1hdHRlc3RhdGlvbi1zYW1wbGVzIiwiUEFUSCI6Ii91c3IvbG9jYWwvc2JpbjovdXNyL2xvY2FsL2JpbjovdXNyL3NiaW46L3Vzci9iaW46L3NiaW46L2JpbiIsIlJVU1RfTE9HIjoiaW5mbyIsIlNTTF9DRVJUX0ZJTEUiOiIvZXRjL3NzbC9jZXJ0cy9jYS1jZXJ0aWZpY2F0ZXMuY3J0In0sImFyZ3MiOlsiL2FwcC9jcy1hdHRlc3RhdGlvbi1zYW1wbGUiXX0sImdjZSI6eyJ6b25lIjoidXMtY2VudHJhbDEtYSIsInByb2plY3RfaWQiOiJzeW5kLWRiLXRlc3RpbmciLCJwcm9qZWN0X251bWJlciI6IjI5Nzk4NTI5Nzg5NCIsImluc3RhbmNlX25hbWUiOiJjcy1hdHRlc3RhdGlvbi12bSIsImluc3RhbmNlX2lkIjoiNTA3MTUyNDIzNDAzNjE3OTg0MSJ9fSwiZ29vZ2xlX3NlcnZpY2VfYWNjb3VudHMiOlsiY3MtYXR0ZXN0YXRpb24td29ya2xvYWRAc3luZC1kYi10ZXN0aW5nLmlhbS5nc2VydmljZWFjY291bnQuY29tIl19.TuEAsdGtioIdU6U1-QZCTZBDFsz19HrEyu3lgLtURLYtm_y_IrX1GaYyrFQ0-SvSYxi301ol0DtqsJ1SB6bb_m16lnalQJMeqxx9fZMIrCYZC4H8OK96s-d-3dYuL6ZCz0hHvvOnm3QXz9oFzlwlr5q-mqtIvvR68YIXWQP5juJL7rpZm85vTdGZ87dilJTItI6sFA-yFHQrP8Y92Mk10LS3drgOg6Q3-OO3UtR7CoRLAUfbZztv9j9kcti-INo_scv9GwkwgMy-0228liTGkjFMhoMgfRL2xeLvaXGP8JhW98hGk0stESqa9MORAZxQ-FSYf-irlnain6jl8K6zPg";

    fn get_test_jwk() -> JwkKey {
        JwkKey {
            alg: "RS256".into(),
            kid: "d6d5071ab75246a42acfa46d29316311cdab51f7".into(),
            kty: "RSA".into(),
            n: "oXx5rKdo3qdKrKo7o5zyV2IQC5p_tKZmOypYuZyyMzQrG1FG22NAxVgNenPb9KlIcXy81w2qfZdhEKXXXlWmQHCf36638V981H4LcBrnRkaK3eQXkX5ojCmREnK2VB7HhBBQg3p0xNzFRclq4s5OcarPWHturxS7kChHwV7Rh9Dhez0vt43sB5YObxmmIUiB5Y373lrgn7uQOtgM6qbJXwThoN5hx5JXQS5OZyiRyGn4PEx460-x5s_q5-ljWuikgsSUSmZwAnf0uXSE-SwrSIaApSVZ2ZhMJA06Md5Io0XYwLUN7itQz6P-BArtfxiCAIrJpZumCY1fXB04kFjaEw".into(),
            e: "AQAB".into(),
            use_: "sig".into(),
        }
    }

    #[test]
    fn test_extract_kid() {
        let kid = extract_kid_from_jwt(SAMPLE_JWT).unwrap();
        assert_eq!(kid, "d6d5071ab75246a42acfa46d29316311cdab51f7");
    }

    #[test]
    fn test_verify_attestation() {
        let jwk = get_test_jwk();

        // Verify without time checks (token is from the past)
        let result = verify_gcp_cs_attestation(SAMPLE_JWT, &jwk, None, None).unwrap();

        assert_eq!(
            result.image_digest,
            "sha256:61bb0cf00789162e9940a97621a6d4e53126638421aea181d8bff39d6e2f21fa"
        );
        assert_eq!(result.audience, "https://synddb-sequencer.example.com");
        assert!(result.secboot);
        assert_eq!(result.hwmodel, "GCP_AMD_SEV");
        assert_eq!(result.dbgstat, "enabled");
        assert_eq!(result.validity_window_start, 1764707757);
        assert_eq!(result.validity_window_end, 1764711357);
    }

    #[test]
    fn test_verify_with_audience() {
        let jwk = get_test_jwk();

        // Should succeed with correct audience
        let result = verify_gcp_cs_attestation(
            SAMPLE_JWT,
            &jwk,
            Some("https://synddb-sequencer.example.com"),
            None,
        );
        assert!(result.is_ok());

        // Should fail with wrong audience
        let result = verify_gcp_cs_attestation(SAMPLE_JWT, &jwk, Some("https://wrong.com"), None);
        assert!(matches!(result, Err(VerificationError::InvalidIssuer(_))));
    }

    #[test]
    fn test_wrong_key() {
        let mut jwk = get_test_jwk();
        jwk.kid = "wrong-kid".into();

        let result = verify_gcp_cs_attestation(SAMPLE_JWT, &jwk, None, None);
        assert!(matches!(result, Err(VerificationError::KeyNotFound(_))));
    }
}
