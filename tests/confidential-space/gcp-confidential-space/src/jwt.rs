//! JWT parsing utilities for GCP Confidential Space attestation tokens

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

/// JWT header structure
#[derive(Debug, Deserialize)]
pub struct JwtHeader {
    /// Algorithm (expected: RS256)
    pub alg: String,
    /// Key ID (matches jwks.keys[].kid)
    pub kid: String,
    /// Token type (expected: JWT)
    pub typ: String,
}

/// GCP Confidential Space attestation claims
///
/// Reference: https://cloud.google.com/confidential-computing/confidential-space/docs/reference/token-claims
#[derive(Debug, Deserialize)]
pub struct GcpCsClaims {
    /// Issuer (must be "https://confidentialcomputing.googleapis.com")
    pub iss: String,

    /// Subject (instance resource path)
    pub sub: String,

    /// Audience (requested by workload)
    pub aud: String,

    /// Expiration time (Unix timestamp)
    pub exp: u64,

    /// Issued at (Unix timestamp)
    pub iat: u64,

    /// Not before (Unix timestamp)
    pub nbf: u64,

    /// Secure boot enabled
    pub secboot: bool,

    /// Hardware model (e.g., "GCP_AMD_SEV")
    pub hwmodel: String,

    /// Software name (expected: "CONFIDENTIAL_SPACE")
    pub swname: String,

    /// Debug status ("enabled" or "disabled")
    pub dbgstat: String,

    /// Nonce (optional, for replay protection)
    #[serde(default)]
    pub eat_nonce: Option<String>,

    /// Sub-modules containing container and GCE info
    pub submods: SubMods,
}

/// Sub-modules in the attestation claims
#[derive(Debug, Deserialize)]
pub struct SubMods {
    /// Container information
    pub container: ContainerInfo,
}

/// Container information from attestation
#[derive(Debug, Deserialize)]
pub struct ContainerInfo {
    /// Container image digest (e.g., "sha256:...")
    pub image_digest: String,

    /// Container image reference
    pub image_reference: String,
}

/// JWK (JSON Web Key) for RSA public key
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwkKey {
    /// Algorithm (expected: RS256)
    pub alg: String,

    /// Key ID
    pub kid: String,

    /// Key type (expected: RSA)
    pub kty: String,

    /// RSA modulus (base64url encoded)
    pub n: String,

    /// RSA exponent (base64url encoded)
    pub e: String,

    /// Key use (expected: sig)
    #[serde(rename = "use")]
    pub use_: String,
}

/// Parsed JWT components
pub struct ParsedJwt {
    /// Decoded header
    pub header: JwtHeader,
    /// Decoded claims
    pub claims: GcpCsClaims,
    /// Raw signature bytes
    pub signature: Vec<u8>,
    /// Signing input (header.payload in base64url)
    pub signing_input: Vec<u8>,
}

/// Decode base64url to bytes (handles missing padding)
pub fn decode_base64url(input: &str) -> Result<Vec<u8>, &'static str> {
    // base64url uses - and _ instead of + and /
    let mut standard = String::with_capacity(input.len() + 4);
    for c in input.chars() {
        match c {
            '-' => standard.push('+'),
            '_' => standard.push('/'),
            c => standard.push(c),
        }
    }

    // Add padding if needed
    match standard.len() % 4 {
        2 => {
            standard.push('=');
            standard.push('=');
        }
        3 => standard.push('='),
        _ => {}
    }

    decode_base64_standard(&standard)
}

/// Simple base64 decoder (standard alphabet with padding)
fn decode_base64_standard(input: &str) -> Result<Vec<u8>, &'static str> {
    const DECODE_TABLE: [i8; 128] = [
        -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
        -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 62, -1, -1,
        -1, 63, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, -1, -1, -1, -2, -1, -1, -1, 0, 1, 2, 3, 4,
        5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, -1, -1, -1,
        -1, -1, -1, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45,
        46, 47, 48, 49, 50, 51, -1, -1, -1, -1, -1,
    ];

    let input = input.as_bytes();
    if input.len() % 4 != 0 {
        return Err("Invalid base64 length");
    }

    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut i = 0;

    while i < input.len() {
        let a = input[i];
        let b = input[i + 1];
        let c = input[i + 2];
        let d = input[i + 3];

        let va = if a < 128 { DECODE_TABLE[a as usize] } else { -1 };
        let vb = if b < 128 { DECODE_TABLE[b as usize] } else { -1 };
        let vc = if c < 128 { DECODE_TABLE[c as usize] } else { -1 };
        let vd = if d < 128 { DECODE_TABLE[d as usize] } else { -1 };

        if va < 0 || vb < 0 {
            return Err("Invalid base64 character");
        }

        output.push(((va as u8) << 2) | ((vb as u8) >> 4));

        if c != b'=' {
            if vc < 0 {
                return Err("Invalid base64 character");
            }
            output.push(((vb as u8) << 4) | ((vc as u8) >> 2));

            if d != b'=' {
                if vd < 0 {
                    return Err("Invalid base64 character");
                }
                output.push(((vc as u8) << 6) | (vd as u8));
            }
        }

        i += 4;
    }

    Ok(output)
}

/// Parse a JWT token into its components
pub fn parse_jwt(token: &[u8]) -> Result<ParsedJwt, &'static str> {
    let token_str = core::str::from_utf8(token).map_err(|_| "Invalid UTF-8 in JWT")?;

    // Split into parts
    let parts: Vec<&str> = token_str.split('.').collect();
    if parts.len() != 3 {
        return Err("JWT must have 3 parts");
    }

    let header_b64 = parts[0];
    let payload_b64 = parts[1];
    let signature_b64 = parts[2];

    // Decode header
    let header_bytes = decode_base64url(header_b64)?;
    let header: JwtHeader =
        serde_json::from_slice(&header_bytes).map_err(|_| "Failed to parse JWT header")?;

    // Decode payload (claims)
    let payload_bytes = decode_base64url(payload_b64)?;
    let claims: GcpCsClaims =
        serde_json::from_slice(&payload_bytes).map_err(|_| "Failed to parse JWT claims")?;

    // Decode signature
    let signature = decode_base64url(signature_b64)?;

    // The signing input is the raw base64url-encoded header.payload
    let signing_input = {
        let mut input = Vec::with_capacity(header_b64.len() + 1 + payload_b64.len());
        input.extend_from_slice(header_b64.as_bytes());
        input.push(b'.');
        input.extend_from_slice(payload_b64.as_bytes());
        input
    };

    Ok(ParsedJwt {
        header,
        claims,
        signature,
        signing_input,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_base64url() {
        // Test basic decoding
        let decoded = decode_base64url("SGVsbG8").unwrap();
        assert_eq!(&decoded, b"Hello");

        // Test with URL-safe characters
        let decoded = decode_base64url("PDw_Pz4-").unwrap();
        assert_eq!(&decoded, b"<<??>>".as_slice());
    }

    #[test]
    fn test_decode_base64_standard() {
        let decoded = decode_base64_standard("SGVsbG8gV29ybGQ=").unwrap();
        assert_eq!(&decoded, b"Hello World");
    }
}
