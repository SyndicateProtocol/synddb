// Only run this as a WASM if the export-abi feature is not set.
#![cfg_attr(not(any(feature = "export-abi", test)), no_main)]
extern crate alloc;

use alloc::vec::Vec;
use alloy_primitives::{Address, FixedBytes, U64};
use alloy_sol_types::{sol, SolValue};
use stylus_sdk::{
    abi::Bytes,
    call,
    prelude::*,
    storage::{StorageAddress, StorageBool, StorageFixedBytes, StorageMap, StorageU64},
};

/// The ecrecover precompile address (0x01)
const ECRECOVER: Address = Address::with_last_byte(0x01);
/// The SHA-256 precompile address (0x02)
const SHA256_PRECOMPILE: Address = Address::with_last_byte(0x02);
/// The modular exponentiation precompile address (0x05)
const MODEXP_PRECOMPILE: Address = Address::with_last_byte(0x05);

/// PKCS#1 v1.5 DigestInfo prefix for SHA-256
/// DER encoding: SEQUENCE { SEQUENCE { OID sha256, NULL }, OCTET STRING (32 bytes) }
const PKCS1_SHA256_DIGEST_INFO: [u8; 19] = [
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01,
    0x05, 0x00, 0x04, 0x20,
];

sol! {
    /// ABI-encoded attestation claims. Must match the Solidity `PublicValuesStruct`
    /// in `RiscZeroAttestationVerifier` for interface compatibility.
    struct PublicValuesStruct {
        bytes32 jwk_key_hash;
        uint64 validity_window_start;
        uint64 validity_window_end;
        bytes32 image_digest_hash;
        address tee_signing_key;
        bool secboot;
        bool dbgstat_disabled;
        bytes32 audience_hash;
        uint8 image_signature_v;
        bytes32 image_signature_r;
        bytes32 image_signature_s;
    }

    /// Proof data containing the raw JWT and JWK RSA key material.
    /// The caller provides the JWK key material; the contract verifies its hash
    /// matches a stored trusted value before using it for RSA verification.
    struct StylusProofData {
        bytes jwt;
        bytes jwk_modulus;
        bytes jwk_exponent;
    }

    event TrustedJwkAdded(bytes32 indexed kid_hash, bytes32 key_material_hash);
    event TrustedJwkRemoved(bytes32 indexed kid_hash);
    event AllowedImageDigestHashAdded(bytes32 indexed digest_hash);
    event AllowedImageDigestHashRemoved(bytes32 indexed digest_hash);
    event TrustedImageSignerAdded(address indexed signer);
    event TrustedImageSignerRemoved(address indexed signer);
    event OwnershipTransferred(address indexed previous_owner, address indexed new_owner);

    error NotOwner(address caller);
    error AlreadyInitialized();
    error InvalidJwt();
    error UnsupportedAlgorithm();
    error UntrustedJwk(bytes32 kid_hash);
    error JwkKeyMaterialMismatch(bytes32 kid_hash);
    error RsaVerificationFailed();
    error InvalidIssuer();
    error ValidityWindowNotStarted(uint64 start, uint64 current);
    error ValidityWindowExpired(uint64 end, uint64 current);
    error ClaimsMismatch();
    error SecureBootRequired();
    error DebugModeNotAllowed();
    error UntrustedImageDigest(bytes32 digest_hash);
    error UntrustedImageSigner(address signer);
    error ImageSignatureInvalid();
    error EcrecoverFailed();
    error InvalidProofData();
    error InvalidPublicValues();
    error PrecompileFailed();
    error ZeroAddress();
    error ToleranceExceedsOneDay();
}

#[derive(SolidityError)]
pub enum VerifierError {
    NotOwner(NotOwner),
    AlreadyInitialized(AlreadyInitialized),
    InvalidJwt(InvalidJwt),
    UnsupportedAlgorithm(UnsupportedAlgorithm),
    UntrustedJwk(UntrustedJwk),
    JwkKeyMaterialMismatch(JwkKeyMaterialMismatch),
    RsaVerificationFailed(RsaVerificationFailed),
    InvalidIssuer(InvalidIssuer),
    ValidityWindowNotStarted(ValidityWindowNotStarted),
    ValidityWindowExpired(ValidityWindowExpired),
    ClaimsMismatch(ClaimsMismatch),
    SecureBootRequired(SecureBootRequired),
    DebugModeNotAllowed(DebugModeNotAllowed),
    UntrustedImageDigest(UntrustedImageDigest),
    UntrustedImageSigner(UntrustedImageSigner),
    ImageSignatureInvalid(ImageSignatureInvalid),
    EcrecoverFailed(EcrecoverFailed),
    InvalidProofData(InvalidProofData),
    InvalidPublicValues(InvalidPublicValues),
    PrecompileFailed(PrecompileFailed),
    ZeroAddress(ZeroAddress),
    ToleranceExceedsOneDay(ToleranceExceedsOneDay),
}

/// Stylus attestation verifier that directly verifies GCP Confidential Space
/// JWT tokens on-chain using RSA signature verification via EVM precompiles.
///
/// Unlike the `RiscZeroAttestationVerifier` which requires an off-chain zkVM proof,
/// this contract performs full JWT verification on-chain:
///
/// 1. Caller provides the raw JWT token and JWK RSA public key material
/// 2. Contract verifies the JWK key material hash matches a stored trusted value
/// 3. Contract verifies the JWT RS256 signature using SHA-256 (0x02) and modexp (0x05)
/// 4. Contract parses and validates all attestation claims from the JWT payload
/// 5. Contract verifies the container image signature via ecrecover (0x01)
/// 6. Returns the TEE signing key address
///
/// Implements the same `IAttestationVerifier` ABI interface as the Solidity verifier,
/// making it a drop-in replacement via `TeeKeyManager.updateAttestationVerifier()`.
#[storage]
#[entrypoint]
pub struct StylusAttestationVerifier {
    owner: StorageAddress,
    initialized: StorageBool,
    expiration_tolerance: StorageU64,
    /// Maps `keccak256(kid)` -> whether this JWK key ID is trusted
    trusted_jwk_hashes: StorageMap<FixedBytes<32>, StorageBool>,
    /// Maps `keccak256(kid)` -> `keccak256(modulus || exponent)` for key material verification.
    /// When a caller provides JWK key material, the contract hashes it and compares against
    /// the stored value to prevent callers from substituting their own RSA keys.
    jwk_key_material_hashes: StorageMap<FixedBytes<32>, StorageFixedBytes<32>>,
    allowed_image_digest_hashes: StorageMap<FixedBytes<32>, StorageBool>,
    trusted_image_signers: StorageMap<Address, StorageBool>,
}

#[public]
impl StylusAttestationVerifier {
    /// Initializes the contract. Must be called once after deployment.
    /// Sets the caller as the owner. Expiration tolerance is capped at 86400 seconds (1 day).
    pub fn initialize(&mut self, expiration_tolerance: u64) -> Result<(), VerifierError> {
        if self.initialized.get() {
            return Err(VerifierError::AlreadyInitialized(AlreadyInitialized {}));
        }
        if expiration_tolerance > 86400 {
            return Err(VerifierError::ToleranceExceedsOneDay(
                ToleranceExceedsOneDay {},
            ));
        }
        self.owner.set(self.vm().msg_sender());
        self.expiration_tolerance.set(U64::from(expiration_tolerance));
        self.initialized.set(true);
        Ok(())
    }

    /// Verifies a GCP Confidential Space JWT attestation directly on-chain.
    ///
    /// ABI-compatible with `IAttestationVerifier.verifyAttestationProof(bytes,bytes)`.
    ///
    /// # Arguments
    /// * `public_values` - ABI-encoded `PublicValuesStruct` with claimed attestation values
    /// * `proof_bytes` - ABI-encoded `StylusProofData` containing the raw JWT and JWK key material
    ///
    /// # Returns
    /// The TEE signing key address from the verified attestation
    pub fn verify_attestation_proof(
        &self,
        public_values: Bytes,
        proof_bytes: Bytes,
    ) -> Result<Address, VerifierError> {
        // Decode inputs
        let values = PublicValuesStruct::abi_decode(&public_values)
            .map_err(|_| VerifierError::InvalidPublicValues(InvalidPublicValues {}))?;
        let proof = StylusProofData::abi_decode(&proof_bytes)
            .map_err(|_| VerifierError::InvalidProofData(InvalidProofData {}))?;

        // Parse JWT into header, payload, signature, and signing input
        let jwt = parse_jwt(&proof.jwt)
            .map_err(|_| VerifierError::InvalidJwt(InvalidJwt {}))?;

        // Verify algorithm is RS256
        let alg = json_get_string(&jwt.header_json, b"alg")
            .ok_or(VerifierError::InvalidJwt(InvalidJwt {}))?;
        if alg != b"RS256" {
            return Err(VerifierError::UnsupportedAlgorithm(UnsupportedAlgorithm {}));
        }

        // Extract kid and verify JWK is trusted
        let kid = json_get_string(&jwt.header_json, b"kid")
            .ok_or(VerifierError::InvalidJwt(InvalidJwt {}))?;
        let kid_hash: FixedBytes<32> = self.vm().native_keccak256(kid);

        if !self.trusted_jwk_hashes.get(kid_hash) {
            return Err(VerifierError::UntrustedJwk(UntrustedJwk { kid_hash }));
        }

        // Verify the provided JWK key material matches the stored hash.
        // This prevents callers from substituting their own RSA key to forge JWTs.
        let mut key_material =
            Vec::with_capacity(proof.jwk_modulus.len() + proof.jwk_exponent.len());
        key_material.extend_from_slice(&proof.jwk_modulus);
        key_material.extend_from_slice(&proof.jwk_exponent);
        let actual_key_hash: FixedBytes<32> = self.vm().native_keccak256(&key_material);

        let stored_key_hash = self.jwk_key_material_hashes.get(kid_hash);
        if stored_key_hash != actual_key_hash {
            return Err(VerifierError::JwkKeyMaterialMismatch(
                JwkKeyMaterialMismatch { kid_hash },
            ));
        }

        // Verify RS256 signature (RSA PKCS#1 v1.5 with SHA-256)
        self.verify_rs256(
            &jwt.signing_input,
            &jwt.signature,
            &proof.jwk_modulus,
            &proof.jwk_exponent,
        )?;

        // -- Validate JWT claims against public_values --

        // Issuer must be Google's Confidential Computing service
        let iss = json_get_string(&jwt.payload_json, b"iss")
            .ok_or(VerifierError::InvalidJwt(InvalidJwt {}))?;
        if iss != b"https://confidentialcomputing.googleapis.com" {
            return Err(VerifierError::InvalidIssuer(InvalidIssuer {}));
        }

        // Timestamps from JWT must match public_values
        let iat = json_get_u64(&jwt.payload_json, b"iat")
            .ok_or(VerifierError::InvalidJwt(InvalidJwt {}))?;
        let exp = json_get_u64(&jwt.payload_json, b"exp")
            .ok_or(VerifierError::InvalidJwt(InvalidJwt {}))?;
        if iat != values.validity_window_start || exp != values.validity_window_end {
            return Err(VerifierError::ClaimsMismatch(ClaimsMismatch {}));
        }

        // Validate validity window against current block timestamp
        let current_timestamp = self.vm().block_timestamp();
        if current_timestamp < values.validity_window_start {
            return Err(VerifierError::ValidityWindowNotStarted(
                ValidityWindowNotStarted {
                    start: values.validity_window_start,
                    current: current_timestamp,
                },
            ));
        }

        let tolerance: u64 = self.expiration_tolerance.get().to();
        if current_timestamp > values.validity_window_end + tolerance {
            return Err(VerifierError::ValidityWindowExpired(
                ValidityWindowExpired {
                    end: values.validity_window_end,
                    current: current_timestamp,
                },
            ));
        }

        // Audience hash must match
        let aud = json_get_string(&jwt.payload_json, b"aud")
            .ok_or(VerifierError::InvalidJwt(InvalidJwt {}))?;
        let aud_hash: FixedBytes<32> = self.vm().native_keccak256(aud);
        if aud_hash != values.audience_hash {
            return Err(VerifierError::ClaimsMismatch(ClaimsMismatch {}));
        }

        // Secure boot
        let secboot = json_get_bool(&jwt.payload_json, b"secboot")
            .ok_or(VerifierError::InvalidJwt(InvalidJwt {}))?;
        if secboot != values.secboot {
            return Err(VerifierError::ClaimsMismatch(ClaimsMismatch {}));
        }
        if !values.secboot {
            return Err(VerifierError::SecureBootRequired(SecureBootRequired {}));
        }

        // Debug status
        let dbgstat = json_get_string(&jwt.payload_json, b"dbgstat");
        let dbgstat_disabled = dbgstat
            .map(|d| d == b"disabled-since-boot")
            .unwrap_or(false);
        if dbgstat_disabled != values.dbgstat_disabled {
            return Err(VerifierError::ClaimsMismatch(ClaimsMismatch {}));
        }
        if !values.dbgstat_disabled {
            return Err(VerifierError::DebugModeNotAllowed(DebugModeNotAllowed {}));
        }

        // Image digest hash: find `image_digest` in the JWT payload (nested under submods.container)
        let image_digest = json_get_string(&jwt.payload_json, b"image_digest")
            .ok_or(VerifierError::InvalidJwt(InvalidJwt {}))?;
        let image_digest_hash: FixedBytes<32> = self.vm().native_keccak256(image_digest);
        if image_digest_hash != values.image_digest_hash {
            return Err(VerifierError::ClaimsMismatch(ClaimsMismatch {}));
        }
        if !self.allowed_image_digest_hashes.get(values.image_digest_hash) {
            return Err(VerifierError::UntrustedImageDigest(UntrustedImageDigest {
                digest_hash: values.image_digest_hash,
            }));
        }

        // JWK key hash must match
        if kid_hash != values.jwk_key_hash {
            return Err(VerifierError::ClaimsMismatch(ClaimsMismatch {}));
        }

        // Verify image signature using ecrecover
        let image_signer = self.ecrecover_call(
            values.image_digest_hash,
            values.image_signature_v,
            values.image_signature_r,
            values.image_signature_s,
        )?;

        if image_signer == Address::ZERO {
            return Err(VerifierError::ImageSignatureInvalid(
                ImageSignatureInvalid {},
            ));
        }

        if !self.trusted_image_signers.get(image_signer) {
            return Err(VerifierError::UntrustedImageSigner(UntrustedImageSigner {
                signer: image_signer,
            }));
        }

        Ok(values.tee_signing_key)
    }

    // -- Admin functions --

    /// Adds a trusted JWK key with its RSA key material hash.
    ///
    /// # Arguments
    /// * `kid_hash` - `keccak256(kid)` where kid is the JWK Key ID from Google's JWKS
    /// * `key_material_hash` - `keccak256(modulus_bytes || exponent_bytes)` where modulus and
    ///   exponent are the base64url-decoded RSA key components from the JWK
    pub fn add_trusted_jwk(
        &mut self,
        kid_hash: FixedBytes<32>,
        key_material_hash: FixedBytes<32>,
    ) -> Result<(), VerifierError> {
        self.only_owner()?;
        self.trusted_jwk_hashes.setter(kid_hash).set(true);
        self.jwk_key_material_hashes
            .setter(kid_hash)
            .set(key_material_hash);
        self.vm().log(TrustedJwkAdded {
            kid_hash,
            key_material_hash,
        });
        Ok(())
    }

    /// Removes a trusted JWK key.
    pub fn remove_trusted_jwk(&mut self, kid_hash: FixedBytes<32>) -> Result<(), VerifierError> {
        self.only_owner()?;
        self.trusted_jwk_hashes.setter(kid_hash).set(false);
        self.jwk_key_material_hashes
            .setter(kid_hash)
            .set(FixedBytes::ZERO);
        self.vm().log(TrustedJwkRemoved { kid_hash });
        Ok(())
    }

    /// Adds an allowed container image digest hash.
    pub fn add_allowed_image_digest_hash(
        &mut self,
        digest_hash: FixedBytes<32>,
    ) -> Result<(), VerifierError> {
        self.only_owner()?;
        self.allowed_image_digest_hashes
            .setter(digest_hash)
            .set(true);
        self.vm().log(AllowedImageDigestHashAdded { digest_hash });
        Ok(())
    }

    /// Removes an allowed container image digest hash.
    pub fn remove_allowed_image_digest_hash(
        &mut self,
        digest_hash: FixedBytes<32>,
    ) -> Result<(), VerifierError> {
        self.only_owner()?;
        self.allowed_image_digest_hashes
            .setter(digest_hash)
            .set(false);
        self.vm().log(AllowedImageDigestHashRemoved { digest_hash });
        Ok(())
    }

    /// Adds a trusted image signer.
    pub fn add_trusted_image_signer(&mut self, signer: Address) -> Result<(), VerifierError> {
        self.only_owner()?;
        self.trusted_image_signers.setter(signer).set(true);
        self.vm().log(TrustedImageSignerAdded { signer });
        Ok(())
    }

    /// Removes a trusted image signer.
    pub fn remove_trusted_image_signer(&mut self, signer: Address) -> Result<(), VerifierError> {
        self.only_owner()?;
        self.trusted_image_signers.setter(signer).set(false);
        self.vm().log(TrustedImageSignerRemoved { signer });
        Ok(())
    }

    /// Transfers ownership of the contract. Reverts if `new_owner` is the zero address.
    pub fn transfer_ownership(&mut self, new_owner: Address) -> Result<(), VerifierError> {
        self.only_owner()?;
        if new_owner == Address::ZERO {
            return Err(VerifierError::ZeroAddress(ZeroAddress {}));
        }
        let previous_owner = self.owner.get();
        self.owner.set(new_owner);
        self.vm().log(OwnershipTransferred {
            previous_owner,
            new_owner,
        });
        Ok(())
    }

    // -- View functions --

    pub fn owner(&self) -> Address {
        self.owner.get()
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.get()
    }

    pub fn expiration_tolerance(&self) -> U64 {
        self.expiration_tolerance.get()
    }

    pub fn is_jwk_trusted(&self, kid_hash: FixedBytes<32>) -> bool {
        self.trusted_jwk_hashes.get(kid_hash)
    }

    pub fn jwk_key_material_hash(&self, kid_hash: FixedBytes<32>) -> FixedBytes<32> {
        self.jwk_key_material_hashes.get(kid_hash)
    }

    pub fn is_image_digest_hash_allowed(&self, digest_hash: FixedBytes<32>) -> bool {
        self.allowed_image_digest_hashes.get(digest_hash)
    }

    pub fn is_image_signer_trusted(&self, signer: Address) -> bool {
        self.trusted_image_signers.get(signer)
    }
}

// -- Internal methods --

impl StylusAttestationVerifier {
    fn only_owner(&self) -> Result<(), VerifierError> {
        let sender = self.vm().msg_sender();
        if sender != self.owner.get() {
            return Err(VerifierError::NotOwner(NotOwner { caller: sender }));
        }
        Ok(())
    }

    /// Calls the ecrecover precompile (0x01) to recover an address from an ECDSA signature.
    fn ecrecover_call(
        &self,
        hash: FixedBytes<32>,
        v: u8,
        r: FixedBytes<32>,
        s: FixedBytes<32>,
    ) -> Result<Address, VerifierError> {
        let mut input = Vec::with_capacity(128);
        input.extend_from_slice(hash.as_slice());
        let mut v_padded = [0u8; 32];
        v_padded[31] = v;
        input.extend_from_slice(&v_padded);
        input.extend_from_slice(r.as_slice());
        input.extend_from_slice(s.as_slice());

        match call::static_call(self.vm(), Call::new(), ECRECOVER, &input) {
            Ok(result) if result.len() >= 32 => Ok(Address::from_slice(&result[12..32])),
            _ => Err(VerifierError::EcrecoverFailed(EcrecoverFailed {})),
        }
    }

    /// Calls the SHA-256 precompile (0x02).
    fn sha256_call(&self, data: &[u8]) -> Result<[u8; 32], VerifierError> {
        match call::static_call(self.vm(), Call::new(), SHA256_PRECOMPILE, data) {
            Ok(result) if result.len() == 32 => {
                let mut hash = [0u8; 32];
                hash.copy_from_slice(&result);
                Ok(hash)
            }
            _ => Err(VerifierError::PrecompileFailed(PrecompileFailed {})),
        }
    }

    /// Calls the modexp precompile (0x05) to compute `base^exponent mod modulus`.
    fn modexp_call(
        &self,
        base: &[u8],
        exponent: &[u8],
        modulus: &[u8],
    ) -> Result<Vec<u8>, VerifierError> {
        // Input format: base_length (32) || exp_length (32) || mod_length (32) || base || exp || mod
        let mut input = Vec::with_capacity(96 + base.len() + exponent.len() + modulus.len());

        let mut len_buf = [0u8; 32];
        len_buf[28..32].copy_from_slice(&(base.len() as u32).to_be_bytes());
        input.extend_from_slice(&len_buf);

        len_buf = [0u8; 32];
        len_buf[28..32].copy_from_slice(&(exponent.len() as u32).to_be_bytes());
        input.extend_from_slice(&len_buf);

        len_buf = [0u8; 32];
        len_buf[28..32].copy_from_slice(&(modulus.len() as u32).to_be_bytes());
        input.extend_from_slice(&len_buf);

        input.extend_from_slice(base);
        input.extend_from_slice(exponent);
        input.extend_from_slice(modulus);

        match call::static_call(self.vm(), Call::new(), MODEXP_PRECOMPILE, &input) {
            Ok(result) => Ok(result),
            Err(_) => Err(VerifierError::PrecompileFailed(PrecompileFailed {})),
        }
    }

    /// Verify an RS256 (RSA PKCS#1 v1.5 with SHA-256) signature.
    fn verify_rs256(
        &self,
        signing_input: &[u8],
        signature: &[u8],
        modulus: &[u8],
        exponent: &[u8],
    ) -> Result<(), VerifierError> {
        // 1. SHA-256 hash of the signing input (header_b64.payload_b64)
        let hash = self.sha256_call(signing_input)?;

        // 2. RSA operation: signature^exponent mod modulus
        let decrypted = self.modexp_call(signature, exponent, modulus)?;

        // 3. Verify PKCS#1 v1.5 padding matches expected hash
        if !verify_pkcs1v15_sha256(&decrypted, &hash) {
            return Err(VerifierError::RsaVerificationFailed(
                RsaVerificationFailed {},
            ));
        }

        Ok(())
    }
}

// -- PKCS#1 v1.5 verification --

/// Verify PKCS#1 v1.5 padding for SHA-256.
///
/// Expected format after RSA decryption:
/// `0x00 0x01 [0xFF padding] 0x00 [DigestInfo_SHA256] [32-byte hash]`
fn verify_pkcs1v15_sha256(decrypted: &[u8], expected_hash: &[u8; 32]) -> bool {
    let len = decrypted.len();
    // Minimum: 2 (header) + 8 (min padding) + 1 (separator) + 19 (DigestInfo) + 32 (hash) = 62
    if len < 62 {
        return false;
    }

    // Check header bytes
    if decrypted[0] != 0x00 || decrypted[1] != 0x01 {
        return false;
    }

    // Find separator (0x00) after 0xFF padding
    let mut separator_pos = None;
    for (i, &byte) in decrypted.iter().enumerate().skip(2) {
        if byte == 0x00 {
            separator_pos = Some(i);
            break;
        }
        if byte != 0xFF {
            return false;
        }
    }

    let separator_pos = match separator_pos {
        Some(pos) => pos,
        None => return false,
    };

    // PKCS#1 v1.5 requires at least 8 bytes of 0xFF padding
    if separator_pos < 10 {
        return false;
    }

    // After separator: DigestInfo (19 bytes) + hash (32 bytes) = 51 bytes
    let digest_start = separator_pos + 1;
    if len - digest_start != PKCS1_SHA256_DIGEST_INFO.len() + 32 {
        return false;
    }

    // Verify DigestInfo prefix
    if decrypted[digest_start..digest_start + PKCS1_SHA256_DIGEST_INFO.len()]
        != PKCS1_SHA256_DIGEST_INFO
    {
        return false;
    }

    // Verify hash
    let hash_start = digest_start + PKCS1_SHA256_DIGEST_INFO.len();
    decrypted[hash_start..hash_start + 32] == *expected_hash
}

// -- JWT parsing --

/// Parsed JWT components
struct ParsedJwt {
    header_json: Vec<u8>,
    payload_json: Vec<u8>,
    signature: Vec<u8>,
    /// The raw signing input: header_b64 + "." + payload_b64
    signing_input: Vec<u8>,
}

/// Parse a JWT token into its components.
/// Splits on '.', base64url-decodes header and payload, and extracts the raw signature.
fn parse_jwt(jwt_bytes: &[u8]) -> Result<ParsedJwt, &'static str> {
    let jwt_str = core::str::from_utf8(jwt_bytes).map_err(|_| "Invalid UTF-8")?;

    // Split into header.payload.signature
    let mut parts = jwt_str.splitn(3, '.');
    let header_b64 = parts.next().ok_or("Missing header")?;
    let payload_b64 = parts.next().ok_or("Missing payload")?;
    let signature_b64 = parts.next().ok_or("Missing signature")?;

    let header_json = base64url_decode(header_b64.as_bytes())?;
    let payload_json = base64url_decode(payload_b64.as_bytes())?;
    let signature = base64url_decode(signature_b64.as_bytes())?;

    // Signing input is the raw base64url-encoded header.payload (not decoded)
    let mut signing_input = Vec::with_capacity(header_b64.len() + 1 + payload_b64.len());
    signing_input.extend_from_slice(header_b64.as_bytes());
    signing_input.push(b'.');
    signing_input.extend_from_slice(payload_b64.as_bytes());

    Ok(ParsedJwt {
        header_json,
        payload_json,
        signature,
        signing_input,
    })
}

// -- Base64url decoding --

const BASE64_DECODE_TABLE: [i8; 128] = [
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 62, -1, -1,
    -1, 63, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, -1, -1, -1, -2, -1, -1, -1, 0, 1, 2, 3, 4,
    5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, -1, -1, -1,
    -1, -1, -1, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45,
    46, 47, 48, 49, 50, 51, -1, -1, -1, -1, -1,
];

/// Decode base64url to bytes, handling missing padding.
fn base64url_decode(input: &[u8]) -> Result<Vec<u8>, &'static str> {
    // Convert base64url to standard base64
    let mut standard = Vec::with_capacity(input.len() + 4);
    for &c in input {
        match c {
            b'-' => standard.push(b'+'),
            b'_' => standard.push(b'/'),
            c => standard.push(c),
        }
    }

    // Add padding
    match standard.len() % 4 {
        2 => {
            standard.push(b'=');
            standard.push(b'=');
        }
        3 => standard.push(b'='),
        _ => {}
    }

    base64_decode(&standard)
}

fn base64_decode(input: &[u8]) -> Result<Vec<u8>, &'static str> {
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

        let va = if a < 128 {
            BASE64_DECODE_TABLE[a as usize]
        } else {
            -1
        };
        let vb = if b < 128 {
            BASE64_DECODE_TABLE[b as usize]
        } else {
            -1
        };
        let vc = if c < 128 {
            BASE64_DECODE_TABLE[c as usize]
        } else {
            -1
        };
        let vd = if d < 128 {
            BASE64_DECODE_TABLE[d as usize]
        } else {
            -1
        };

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

// -- Minimal JSON extraction --
//
// These functions extract specific claim values from JWT JSON payloads without
// a full JSON parser. They search for `"key":` patterns and parse the subsequent value.
// This is safe for GCP Confidential Space tokens where claim keys are unique.

/// Find a JSON string value for a given key.
/// Returns the raw bytes between the quotes (no unescaping).
fn json_get_string<'a>(json: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let after_colon = json_find_key(json, key)?;

    // Skip whitespace after colon
    let mut i = after_colon;
    while i < json.len() && is_json_whitespace(json[i]) {
        i += 1;
    }

    // Expect opening quote
    if i >= json.len() || json[i] != b'"' {
        return None;
    }
    i += 1;
    let start = i;

    // Find closing quote (handle escaped quotes correctly, including \\")
    while i < json.len() {
        if json[i] == b'"' {
            // Count consecutive backslashes before this quote.
            // An even number means the quote is NOT escaped.
            let mut backslash_count = 0;
            let mut j = i;
            while j > start && json[j - 1] == b'\\' {
                backslash_count += 1;
                j -= 1;
            }
            if backslash_count % 2 == 0 {
                return Some(&json[start..i]);
            }
        }
        i += 1;
    }

    None
}

/// Find a JSON unsigned integer value for a given key.
fn json_get_u64(json: &[u8], key: &[u8]) -> Option<u64> {
    let after_colon = json_find_key(json, key)?;

    let mut i = after_colon;
    while i < json.len() && is_json_whitespace(json[i]) {
        i += 1;
    }

    let mut result: u64 = 0;
    let mut found = false;
    while i < json.len() && json[i].is_ascii_digit() {
        result = result.checked_mul(10)?.checked_add((json[i] - b'0') as u64)?;
        i += 1;
        found = true;
    }

    if found {
        Some(result)
    } else {
        None
    }
}

/// Find a JSON boolean value for a given key.
fn json_get_bool(json: &[u8], key: &[u8]) -> Option<bool> {
    let after_colon = json_find_key(json, key)?;

    let mut i = after_colon;
    while i < json.len() && is_json_whitespace(json[i]) {
        i += 1;
    }

    if json.len() >= i + 4 && &json[i..i + 4] == b"true" {
        Some(true)
    } else if json.len() >= i + 5 && &json[i..i + 5] == b"false" {
        Some(false)
    } else {
        None
    }
}

/// Find the position immediately after the colon for a given JSON key.
/// Handles the case where a key appears as a substring of a value by verifying
/// the colon follows the closing quote of the key.
fn json_find_key(json: &[u8], key: &[u8]) -> Option<usize> {
    // Build search pattern: "key"
    let mut pattern = Vec::with_capacity(key.len() + 2);
    pattern.push(b'"');
    pattern.extend_from_slice(key);
    pattern.push(b'"');

    let mut search_start = 0;
    loop {
        // Find next occurrence of "key"
        let pos = find_subsequence(&json[search_start..], &pattern)?;
        let abs_pos = search_start + pos;
        let after_pattern = abs_pos + pattern.len();

        // Skip whitespace and look for colon
        let mut i = after_pattern;
        while i < json.len() && is_json_whitespace(json[i]) {
            i += 1;
        }

        if i < json.len() && json[i] == b':' {
            return Some(i + 1);
        }

        // Not a key (probably inside a value), continue searching
        search_start = abs_pos + 1;
        if search_start >= json.len() {
            return None;
        }
    }
}

/// Find the first occurrence of `needle` in `haystack`.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn is_json_whitespace(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b'\n' || b == b'\r'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64url_decode() {
        let decoded = base64url_decode(b"SGVsbG8").unwrap();
        assert_eq!(&decoded, b"Hello");

        let decoded = base64url_decode(b"PDw_Pz4-").unwrap();
        assert_eq!(&decoded, b"<<??>>".as_slice());
    }

    #[test]
    fn test_base64_decode_standard() {
        let decoded = base64_decode(b"SGVsbG8gV29ybGQ=").unwrap();
        assert_eq!(&decoded, b"Hello World");
    }

    #[test]
    fn test_json_get_string() {
        let json = br#"{"iss":"https://confidentialcomputing.googleapis.com","aud":"test"}"#;
        let iss = json_get_string(json, b"iss").unwrap();
        assert_eq!(iss, b"https://confidentialcomputing.googleapis.com");

        let aud = json_get_string(json, b"aud").unwrap();
        assert_eq!(aud, b"test");
    }

    #[test]
    fn test_json_get_string_with_spaces() {
        let json = br#"{ "key" : "value" }"#;
        let val = json_get_string(json, b"key").unwrap();
        assert_eq!(val, b"value");
    }

    #[test]
    fn test_json_get_u64() {
        let json = br#"{"iat":1764707757,"exp":1764711357}"#;
        assert_eq!(json_get_u64(json, b"iat"), Some(1764707757));
        assert_eq!(json_get_u64(json, b"exp"), Some(1764711357));
    }

    #[test]
    fn test_json_get_bool() {
        let json = br#"{"secboot":true,"other":false}"#;
        assert_eq!(json_get_bool(json, b"secboot"), Some(true));
        assert_eq!(json_get_bool(json, b"other"), Some(false));
    }

    #[test]
    fn test_json_get_nested_image_digest() {
        let json = br#"{"submods":{"container":{"image_digest":"sha256:61bb0cf00789","image_id":"sha256:daa1d4c16f8f"}}}"#;
        let digest = json_get_string(json, b"image_digest").unwrap();
        assert_eq!(digest, b"sha256:61bb0cf00789");
    }

    #[test]
    fn test_json_key_in_value_not_matched() {
        // "iss" appears as a substring in the value of "data", but json_find_key should skip it
        let json = br#"{"data":"missing","iss":"correct"}"#;
        let iss = json_get_string(json, b"iss").unwrap();
        assert_eq!(iss, b"correct");
    }

    #[test]
    fn test_parse_jwt_structure() {
        // Create a minimal JWT: base64url("header") + "." + base64url("payload") + "." + base64url("sig")
        let header = base64url_encode(br#"{"alg":"RS256","kid":"test-kid"}"#);
        let payload = base64url_encode(br#"{"iss":"test","iat":1000,"exp":2000}"#);
        let sig = base64url_encode(b"fake-signature");
        let jwt = format!("{}.{}.{}", header, payload, sig);

        let parsed = parse_jwt(jwt.as_bytes()).unwrap();
        assert_eq!(
            json_get_string(&parsed.header_json, b"alg").unwrap(),
            b"RS256"
        );
        assert_eq!(
            json_get_string(&parsed.header_json, b"kid").unwrap(),
            b"test-kid"
        );
        assert_eq!(
            json_get_string(&parsed.payload_json, b"iss").unwrap(),
            b"test"
        );
        assert_eq!(json_get_u64(&parsed.payload_json, b"iat"), Some(1000));
        assert_eq!(json_get_u64(&parsed.payload_json, b"exp"), Some(2000));
        assert_eq!(parsed.signature, b"fake-signature");

        // Signing input should be header_b64.payload_b64
        let expected_signing_input = format!("{}.{}", header, payload);
        assert_eq!(parsed.signing_input, expected_signing_input.as_bytes());
    }

    /// Helper to build a PKCS#1 v1.5 padded message.
    /// Format: 0x00 || 0x01 || [0xFF * ff_count] || 0x00 || digest_info || hash
    fn build_pkcs1v15_padded(header: [u8; 2], ff_count: usize, hash: &[u8; 32]) -> Vec<u8> {
        let mut padded = Vec::with_capacity(header.len() + ff_count + 1 + PKCS1_SHA256_DIGEST_INFO.len() + 32);
        padded.extend_from_slice(&header);
        padded.extend(core::iter::repeat_n(0xFF, ff_count));
        padded.push(0x00);
        padded.extend_from_slice(&PKCS1_SHA256_DIGEST_INFO);
        padded.extend_from_slice(hash);
        padded
    }

    #[test]
    fn test_pkcs1v15_sha256_valid() {
        // Build a valid PKCS#1 v1.5 padded message for a 256-byte RSA key
        let hash = [0xAB; 32];
        // 256 - 2 - 1 - 19 - 32 = 202 bytes of 0xFF
        let padded = build_pkcs1v15_padded([0x00, 0x01], 202, &hash);
        assert_eq!(padded.len(), 256);

        assert!(verify_pkcs1v15_sha256(&padded, &hash));
    }

    #[test]
    fn test_pkcs1v15_sha256_wrong_hash() {
        let hash = [0xAB; 32];
        let wrong_hash = [0xCD; 32];
        let padded = build_pkcs1v15_padded([0x00, 0x01], 202, &hash);

        assert!(!verify_pkcs1v15_sha256(&padded, &wrong_hash));
    }

    #[test]
    fn test_pkcs1v15_sha256_bad_header() {
        let hash = [0xAB; 32];
        let padded = build_pkcs1v15_padded([0x00, 0x02], 202, &hash); // Wrong: should be 0x01

        assert!(!verify_pkcs1v15_sha256(&padded, &hash));
    }

    #[test]
    fn test_pkcs1v15_sha256_insufficient_padding() {
        let hash = [0xAB; 32];
        // Only 3 bytes of 0xFF padding (minimum is 8)
        let mut padded = vec![0x00, 0x01, 0xFF, 0xFF, 0xFF, 0x00];
        padded.extend_from_slice(&PKCS1_SHA256_DIGEST_INFO);
        padded.extend_from_slice(&hash);

        assert!(!verify_pkcs1v15_sha256(&padded, &hash));
    }

    #[test]
    fn test_public_values_abi_roundtrip() {
        let values = PublicValuesStruct {
            jwk_key_hash: FixedBytes::ZERO,
            validity_window_start: 1000,
            validity_window_end: 2000,
            image_digest_hash: FixedBytes::ZERO,
            tee_signing_key: Address::ZERO,
            secboot: true,
            dbgstat_disabled: true,
            audience_hash: FixedBytes::ZERO,
            image_signature_v: 27,
            image_signature_r: FixedBytes::ZERO,
            image_signature_s: FixedBytes::ZERO,
        };

        let encoded = values.abi_encode();
        let decoded = PublicValuesStruct::abi_decode(&encoded).unwrap();

        assert_eq!(decoded.validity_window_start, 1000);
        assert_eq!(decoded.validity_window_end, 2000);
        assert!(decoded.secboot);
        assert!(decoded.dbgstat_disabled);
        assert_eq!(decoded.image_signature_v, 27);
    }

    #[test]
    fn test_stylus_proof_data_abi_roundtrip() {
        let proof = StylusProofData {
            jwt: alloy_primitives::Bytes::from_static(b"jwt-data"),
            jwk_modulus: alloy_primitives::Bytes::from_static(b"modulus-data"),
            jwk_exponent: alloy_primitives::Bytes::from_static(b"exponent-data"),
        };

        let encoded = proof.abi_encode();
        let decoded = StylusProofData::abi_decode(&encoded).unwrap();

        assert_eq!(decoded.jwt.as_ref(), b"jwt-data");
        assert_eq!(decoded.jwk_modulus.as_ref(), b"modulus-data");
        assert_eq!(decoded.jwk_exponent.as_ref(), b"exponent-data");
    }

    // -- Base64 edge case tests --

    #[test]
    fn test_base64url_decode_empty() {
        let decoded = base64url_decode(b"").unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_base64url_decode_single_byte() {
        // Single byte (0x41 = 'A') encodes to "QQ" in base64url (2 chars, needs == padding)
        let decoded = base64url_decode(b"QQ").unwrap();
        assert_eq!(decoded, vec![0x41]);
    }

    #[test]
    fn test_base64url_decode_two_bytes() {
        // Two bytes encode to 3 base64url chars (needs = padding)
        let decoded = base64url_decode(b"QUI").unwrap();
        assert_eq!(decoded, vec![0x41, 0x42]);
    }

    #[test]
    fn test_base64url_decode_invalid_char() {
        // Space is not a valid base64 character
        assert!(base64url_decode(b"QQ Q").is_err());
    }

    #[test]
    fn test_base64_decode_invalid_length() {
        // Length not multiple of 4 after padding
        assert!(base64_decode(b"QQQQ Q").is_err());
    }

    #[test]
    fn test_base64url_decode_all_url_safe_chars() {
        // Test that both - and _ are correctly translated
        // "+" in standard base64 = "-" in base64url
        // "/" in standard base64 = "_" in base64url
        let decoded = base64url_decode(b"-_8").unwrap();
        // "-" -> "+", "_" -> "/", "8" stays; "+" = 62, "/" = 63, "8" = 60
        // After decoding: these are specific byte values
        let standard_decoded = base64_decode(b"+/8=").unwrap();
        assert_eq!(decoded, standard_decoded);
    }

    #[test]
    fn test_base64_decode_high_byte_rejected() {
        // Bytes >= 128 should be rejected
        let input = [0x80, b'Q', b'Q', b'Q'];
        assert!(base64_decode(&input).is_err());
    }

    // -- JSON extraction edge case tests --

    #[test]
    fn test_json_get_string_missing_key() {
        let json = br#"{"iss":"test"}"#;
        assert!(json_get_string(json, b"nonexistent").is_none());
    }

    #[test]
    fn test_json_get_string_empty_value() {
        let json = br#"{"key":""}"#;
        let val = json_get_string(json, b"key").unwrap();
        assert_eq!(val, b"");
    }

    #[test]
    fn test_json_get_string_with_escaped_quote() {
        // JSON: {"key":"value\"with\"quotes","other":"ok"}
        // The value contains escaped quotes: value"with"quotes
        let json = br#"{"key":"value\"with\"quotes","other":"ok"}"#;
        let val = json_get_string(json, b"key").unwrap();
        // Raw bytes between the outer quotes include the backslash-quote sequences
        assert_eq!(val, br#"value\"with\"quotes"#);
    }

    #[test]
    fn test_json_get_string_with_double_backslash_before_quote() {
        // JSON: {"key":"value\\","other":"ok"}
        // The \\\\ in the raw string is two backslashes in the actual bytes.
        // The first backslash escapes the second, so the quote after them is the real closing quote.
        let json = br#"{"key":"value\\","other":"ok"}"#;
        let val = json_get_string(json, b"key").unwrap();
        assert_eq!(val, br#"value\\"#);

        // Verify the next key is still extractable
        let other = json_get_string(json, b"other").unwrap();
        assert_eq!(other, b"ok");
    }

    #[test]
    fn test_json_get_u64_missing_key() {
        let json = br#"{"iat":1000}"#;
        assert!(json_get_u64(json, b"exp").is_none());
    }

    #[test]
    fn test_json_get_u64_zero() {
        let json = br#"{"val":0}"#;
        assert_eq!(json_get_u64(json, b"val"), Some(0));
    }

    #[test]
    fn test_json_get_u64_max_safe() {
        // Large but valid u64 value
        let json = br#"{"big":18446744073709551615}"#;
        assert_eq!(json_get_u64(json, b"big"), Some(u64::MAX));
    }

    #[test]
    fn test_json_get_u64_overflow() {
        // u64::MAX + 1 should overflow and return None
        let json = br#"{"big":18446744073709551616}"#;
        assert!(json_get_u64(json, b"big").is_none());
    }

    #[test]
    fn test_json_get_u64_not_a_number() {
        let json = br#"{"val":"not_a_number"}"#;
        assert!(json_get_u64(json, b"val").is_none());
    }

    #[test]
    fn test_json_get_bool_missing_key() {
        let json = br#"{"secboot":true}"#;
        assert!(json_get_bool(json, b"nonexistent").is_none());
    }

    #[test]
    fn test_json_get_bool_not_a_bool() {
        // Value is a string, not a bool literal
        let json = br#"{"flag":"true"}"#;
        assert!(json_get_bool(json, b"flag").is_none());
    }

    #[test]
    fn test_json_get_string_key_at_end() {
        let json = br#"{"first":"a","last":"z"}"#;
        assert_eq!(json_get_string(json, b"last").unwrap(), b"z");
    }

    #[test]
    fn test_json_get_string_value_contains_colon() {
        let json = br#"{"url":"https://example.com:8080/path"}"#;
        assert_eq!(
            json_get_string(json, b"url").unwrap(),
            b"https://example.com:8080/path"
        );
    }

    #[test]
    fn test_json_get_string_key_substring_of_another() {
        // "aud" is a substring of "audience" - should only match exact key
        let json = br#"{"audience":"wrong","aud":"correct"}"#;
        assert_eq!(json_get_string(json, b"aud").unwrap(), b"correct");
    }

    #[test]
    fn test_json_find_key_value_looks_like_key() {
        // The value of "description" contains "iss": which looks like a key-value pair
        let json = br#"{"description":"the iss is important","iss":"real_issuer"}"#;
        let iss = json_get_string(json, b"iss").unwrap();
        assert_eq!(iss, b"real_issuer");
    }

    #[test]
    fn test_json_get_nested_deeply() {
        // Test deeply nested object extraction
        let json = br#"{"a":{"b":{"c":{"target":"found"}}}}"#;
        assert_eq!(json_get_string(json, b"target").unwrap(), b"found");
    }

    #[test]
    fn test_json_get_u64_followed_by_comma() {
        let json = br#"{"a":123,"b":456}"#;
        assert_eq!(json_get_u64(json, b"a"), Some(123));
        assert_eq!(json_get_u64(json, b"b"), Some(456));
    }

    #[test]
    fn test_json_get_u64_followed_by_brace() {
        let json = br#"{"val":789}"#;
        assert_eq!(json_get_u64(json, b"val"), Some(789));
    }

    // -- JWT parsing error tests --

    #[test]
    fn test_parse_jwt_invalid_utf8() {
        let invalid = vec![0xFF, 0xFE, 0xFD];
        assert!(parse_jwt(&invalid).is_err());
    }

    #[test]
    fn test_parse_jwt_no_dots() {
        assert!(parse_jwt(b"nodots").is_err());
    }

    #[test]
    fn test_parse_jwt_one_dot() {
        assert!(parse_jwt(b"only.one").is_err());
    }

    #[test]
    fn test_parse_jwt_invalid_base64_in_header() {
        // "!!!" is not valid base64
        assert!(parse_jwt(b"!!!.cGF5bG9hZA.c2ln").is_err());
    }

    #[test]
    fn test_parse_jwt_empty_parts() {
        // Three dots but empty segments - base64url_decode of "" returns empty vec, which is valid
        let result = parse_jwt(b"..");
        // Empty header/payload decode to empty JSON, which is fine at the parsing level
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_jwt_extra_dots_ignored() {
        // JWT with extra dots - splitn(3, '.') means the third part includes everything after second dot
        let header = base64url_encode(br#"{"alg":"RS256"}"#);
        let payload = base64url_encode(br#"{"iss":"test"}"#);
        let jwt = format!("{}.{}.sig.extra.dots", header, payload);
        // The signature part will be "sig.extra.dots" which will fail base64 decode
        // because '.' is not valid base64
        assert!(parse_jwt(jwt.as_bytes()).is_err());
    }

    #[test]
    fn test_parse_jwt_preserves_signing_input_exactly() {
        // The signing input must be the exact base64url-encoded header.payload
        let header_b64 = base64url_encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let payload_b64 = base64url_encode(br#"{"sub":"test"}"#);
        let sig_b64 = base64url_encode(b"\x01\x02\x03");
        let jwt = format!("{}.{}.{}", header_b64, payload_b64, sig_b64);

        let parsed = parse_jwt(jwt.as_bytes()).unwrap();
        let expected = format!("{}.{}", header_b64, payload_b64);
        assert_eq!(
            core::str::from_utf8(&parsed.signing_input).unwrap(),
            &expected
        );
    }

    // -- PKCS#1 v1.5 additional edge case tests --

    #[test]
    fn test_pkcs1v15_sha256_non_ff_in_padding() {
        // A non-0xFF byte (0xFE) in the padding should fail
        let hash = [0xAB; 32];
        let mut padded = vec![0x00, 0x01];
        padded.extend(core::iter::repeat_n(0xFF, 100));
        padded.push(0xFE); // Invalid: not 0xFF
        padded.extend(core::iter::repeat_n(0xFF, 101));
        padded.push(0x00);
        padded.extend_from_slice(&PKCS1_SHA256_DIGEST_INFO);
        padded.extend_from_slice(&hash);

        assert!(!verify_pkcs1v15_sha256(&padded, &hash));
    }

    #[test]
    fn test_pkcs1v15_sha256_wrong_digest_info() {
        let hash = [0xAB; 32];
        let mut padded = vec![0x00, 0x01];
        padded.extend(core::iter::repeat_n(0xFF, 202));
        padded.push(0x00);
        // Wrong DigestInfo - change first byte
        let mut wrong_digest_info = PKCS1_SHA256_DIGEST_INFO;
        wrong_digest_info[0] = 0x31; // Should be 0x30
        padded.extend_from_slice(&wrong_digest_info);
        padded.extend_from_slice(&hash);

        assert!(!verify_pkcs1v15_sha256(&padded, &hash));
    }

    #[test]
    fn test_pkcs1v15_sha256_too_short() {
        // Less than 62 bytes should fail immediately
        let hash = [0xAB; 32];
        let padded = vec![0x00, 0x01, 0xFF, 0x00]; // Way too short
        assert!(!verify_pkcs1v15_sha256(&padded, &hash));
    }

    #[test]
    fn test_pkcs1v15_sha256_minimum_valid_padding() {
        // Exactly 8 bytes of 0xFF padding (minimum per PKCS#1 v1.5 spec)
        let hash = [0xAB; 32];
        let padded = build_pkcs1v15_padded([0x00, 0x01], 8, &hash);
        // 2 + 8 + 1 + 19 + 32 = 62 bytes (minimum valid)
        assert_eq!(padded.len(), 62);

        assert!(verify_pkcs1v15_sha256(&padded, &hash));
    }

    #[test]
    fn test_pkcs1v15_sha256_128_byte_key() {
        // 1024-bit RSA key (128 bytes) - valid but smaller padding
        let hash = [0xAB; 32];
        // 128 - 2 - 1 - 19 - 32 = 74 bytes of 0xFF
        let padded = build_pkcs1v15_padded([0x00, 0x01], 74, &hash);
        assert_eq!(padded.len(), 128);

        assert!(verify_pkcs1v15_sha256(&padded, &hash));
    }

    #[test]
    fn test_pkcs1v15_sha256_no_separator() {
        // All 0xFF with no 0x00 separator - should fail
        let hash = [0xAB; 32];
        let mut padded = vec![0x00, 0x01];
        padded.extend(core::iter::repeat_n(0xFF, 256));
        // No separator, no DigestInfo, no hash
        assert!(!verify_pkcs1v15_sha256(&padded, &hash));
    }

    #[test]
    fn test_pkcs1v15_sha256_extra_data_after_hash() {
        // Correct padding but extra bytes after the hash (wrong total length)
        let hash = [0xAB; 32];
        let mut padded = build_pkcs1v15_padded([0x00, 0x01], 202, &hash);
        padded.push(0x00); // Extra byte - makes total length wrong
        assert_eq!(padded.len(), 257);

        // The check `len - digest_start != 51` should catch this
        assert!(!verify_pkcs1v15_sha256(&padded, &hash));
    }

    // -- Realistic GCP Confidential Space JWT parsing test --

    #[test]
    fn test_parse_realistic_gcp_jwt() {
        // Build a realistic GCP Confidential Space JWT payload
        let header = br#"{"alg":"RS256","kid":"1a2b3c4d5e6f","typ":"JWT"}"#;
        let payload = br#"{"iss":"https://confidentialcomputing.googleapis.com","sub":"https://www.googleapis.com/compute/v1/projects/my-project/zones/us-central1-a/instances/12345","aud":"https://synddb.example.com","iat":1764707757,"exp":1764711357,"nbf":1764707757,"secboot":true,"hwmodel":"GCP_AMD_SEV","swname":"CONFIDENTIAL_SPACE","dbgstat":"disabled-since-boot","submods":{"container":{"image_digest":"sha256:61bb0cf00789abcdef1234567890abcdef1234567890abcdef1234567890abcd","image_reference":"us-central1-docker.pkg.dev/my-project/my-repo/sequencer@sha256:61bb0cf00789abcdef1234567890abcdef1234567890abcdef1234567890abcd"}}}"#;
        let sig = b"fake-rsa-signature-for-testing";

        let header_b64 = base64url_encode(header);
        let payload_b64 = base64url_encode(payload);
        let sig_b64 = base64url_encode(sig);
        let jwt = format!("{}.{}.{}", header_b64, payload_b64, sig_b64);

        let parsed = parse_jwt(jwt.as_bytes()).unwrap();

        // Verify header fields
        assert_eq!(
            json_get_string(&parsed.header_json, b"alg").unwrap(),
            b"RS256"
        );
        assert_eq!(
            json_get_string(&parsed.header_json, b"kid").unwrap(),
            b"1a2b3c4d5e6f"
        );
        assert_eq!(
            json_get_string(&parsed.header_json, b"typ").unwrap(),
            b"JWT"
        );

        // Verify payload claims
        assert_eq!(
            json_get_string(&parsed.payload_json, b"iss").unwrap(),
            b"https://confidentialcomputing.googleapis.com"
        );
        assert_eq!(
            json_get_string(&parsed.payload_json, b"aud").unwrap(),
            b"https://synddb.example.com"
        );
        assert_eq!(json_get_u64(&parsed.payload_json, b"iat"), Some(1764707757));
        assert_eq!(json_get_u64(&parsed.payload_json, b"exp"), Some(1764711357));
        assert_eq!(json_get_u64(&parsed.payload_json, b"nbf"), Some(1764707757));
        assert_eq!(json_get_bool(&parsed.payload_json, b"secboot"), Some(true));
        assert_eq!(
            json_get_string(&parsed.payload_json, b"hwmodel").unwrap(),
            b"GCP_AMD_SEV"
        );
        assert_eq!(
            json_get_string(&parsed.payload_json, b"swname").unwrap(),
            b"CONFIDENTIAL_SPACE"
        );
        assert_eq!(
            json_get_string(&parsed.payload_json, b"dbgstat").unwrap(),
            b"disabled-since-boot"
        );
        assert_eq!(
            json_get_string(&parsed.payload_json, b"image_digest").unwrap(),
            b"sha256:61bb0cf00789abcdef1234567890abcdef1234567890abcdef1234567890abcd"
        );

        // Verify signature was decoded
        assert_eq!(parsed.signature, sig);
    }

    // -- Modexp input format verification --

    #[test]
    fn test_modexp_input_construction() {
        // Verify the modexp input format matches the EIP-198 spec:
        // base_length (32 bytes) || exp_length (32 bytes) || mod_length (32 bytes) || base || exp || mod
        let base = vec![0x01; 256]; // 256-byte signature
        let exponent = vec![0x01, 0x00, 0x01]; // Common RSA exponent (65537)
        let modulus = vec![0x02; 256]; // 256-byte modulus

        // Reconstruct what modexp_call would build (without calling the precompile)
        let mut input = Vec::with_capacity(96 + base.len() + exponent.len() + modulus.len());

        let mut len_buf = [0u8; 32];
        len_buf[28..32].copy_from_slice(&(base.len() as u32).to_be_bytes());
        input.extend_from_slice(&len_buf);

        len_buf = [0u8; 32];
        len_buf[28..32].copy_from_slice(&(exponent.len() as u32).to_be_bytes());
        input.extend_from_slice(&len_buf);

        len_buf = [0u8; 32];
        len_buf[28..32].copy_from_slice(&(modulus.len() as u32).to_be_bytes());
        input.extend_from_slice(&len_buf);

        input.extend_from_slice(&base);
        input.extend_from_slice(&exponent);
        input.extend_from_slice(&modulus);

        // Verify total length: 96 (headers) + 256 (base) + 3 (exp) + 256 (mod) = 611
        assert_eq!(input.len(), 611);

        // Verify base_length = 256 (big-endian in last 4 bytes of first 32-byte word)
        assert_eq!(input[28..32], [0x00, 0x00, 0x01, 0x00]);

        // Verify exp_length = 3
        assert_eq!(input[60..64], [0x00, 0x00, 0x00, 0x03]);

        // Verify mod_length = 256
        assert_eq!(input[92..96], [0x00, 0x00, 0x01, 0x00]);

        // Verify base starts at offset 96
        assert_eq!(&input[96..98], &[0x01, 0x01]);

        // Verify exponent starts at offset 96 + 256 = 352
        assert_eq!(&input[352..355], &[0x01, 0x00, 0x01]);

        // Verify modulus starts at offset 352 + 3 = 355
        assert_eq!(&input[355..357], &[0x02, 0x02]);
    }

    // -- ecrecover input format verification --

    #[test]
    fn test_ecrecover_input_construction() {
        // Verify the ecrecover input format: hash (32) || v (32, right-padded) || r (32) || s (32)
        let hash = FixedBytes::<32>::from([0xAA; 32]);
        let v: u8 = 28;
        let r = FixedBytes::<32>::from([0xBB; 32]);
        let s = FixedBytes::<32>::from([0xCC; 32]);

        // Reconstruct what ecrecover_call would build
        let mut input = Vec::with_capacity(128);
        input.extend_from_slice(hash.as_slice());
        let mut v_padded = [0u8; 32];
        v_padded[31] = v;
        input.extend_from_slice(&v_padded);
        input.extend_from_slice(r.as_slice());
        input.extend_from_slice(s.as_slice());

        assert_eq!(input.len(), 128);

        // Hash at bytes 0..32
        assert_eq!(&input[0..32], &[0xAA; 32]);

        // V at byte 63 (right-aligned in 32-byte word)
        assert_eq!(input[31 + 1..63], [0u8; 31]);
        assert_eq!(input[63], 28);

        // R at bytes 64..96
        assert_eq!(&input[64..96], &[0xBB; 32]);

        // S at bytes 96..128
        assert_eq!(&input[96..128], &[0xCC; 32]);
    }

    // -- ABI encoding with non-zero values --

    #[test]
    fn test_public_values_abi_roundtrip_with_real_values() {
        let values = PublicValuesStruct {
            jwk_key_hash: FixedBytes::from([0x11; 32]),
            validity_window_start: 1764707757,
            validity_window_end: 1764711357,
            image_digest_hash: FixedBytes::from([0x22; 32]),
            tee_signing_key: Address::from([0x33; 20]),
            secboot: true,
            dbgstat_disabled: true,
            audience_hash: FixedBytes::from([0x44; 32]),
            image_signature_v: 28,
            image_signature_r: FixedBytes::from([0x55; 32]),
            image_signature_s: FixedBytes::from([0x66; 32]),
        };

        let encoded = values.abi_encode();
        let decoded = PublicValuesStruct::abi_decode(&encoded).unwrap();

        assert_eq!(decoded.jwk_key_hash, FixedBytes::from([0x11; 32]));
        assert_eq!(decoded.validity_window_start, 1764707757);
        assert_eq!(decoded.validity_window_end, 1764711357);
        assert_eq!(decoded.image_digest_hash, FixedBytes::from([0x22; 32]));
        assert_eq!(decoded.tee_signing_key, Address::from([0x33; 20]));
        assert!(decoded.secboot);
        assert!(decoded.dbgstat_disabled);
        assert_eq!(decoded.audience_hash, FixedBytes::from([0x44; 32]));
        assert_eq!(decoded.image_signature_v, 28);
        assert_eq!(decoded.image_signature_r, FixedBytes::from([0x55; 32]));
        assert_eq!(decoded.image_signature_s, FixedBytes::from([0x66; 32]));
    }

    #[test]
    fn test_public_values_abi_decode_invalid_data() {
        // Too short to be valid ABI-encoded data
        assert!(PublicValuesStruct::abi_decode(&[0u8; 10]).is_err());
    }

    #[test]
    fn test_stylus_proof_data_abi_decode_invalid_data() {
        assert!(StylusProofData::abi_decode(&[0u8; 10]).is_err());
    }

    // -- find_subsequence tests --

    #[test]
    fn test_find_subsequence_found() {
        assert_eq!(find_subsequence(b"hello world", b"world"), Some(6));
    }

    #[test]
    fn test_find_subsequence_not_found() {
        assert_eq!(find_subsequence(b"hello world", b"xyz"), None);
    }

    #[test]
    fn test_find_subsequence_needle_larger_than_haystack() {
        assert_eq!(find_subsequence(b"hi", b"hello"), None);
    }

    #[test]
    #[should_panic(expected = "window size must be non-zero")]
    fn test_find_subsequence_empty_needle_panics() {
        // Empty needle causes a panic via .windows(0) - this is fine since
        // json_find_key always builds patterns of at least 3 bytes ("key")
        let _ = find_subsequence(b"hello", b"");
    }

    #[test]
    fn test_find_subsequence_at_start() {
        assert_eq!(find_subsequence(b"hello", b"hel"), Some(0));
    }

    #[test]
    fn test_find_subsequence_at_end() {
        assert_eq!(find_subsequence(b"hello", b"llo"), Some(2));
    }

    /// Helper for tests: encode bytes to base64url (no padding)
    fn base64url_encode(input: &[u8]) -> String {
        const TABLE: &[u8] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut result = String::with_capacity(input.len().div_ceil(3) * 4);
        for chunk in input.chunks(3) {
            let b0 = chunk[0] as usize;
            let b1 = if chunk.len() > 1 { chunk[1] as usize } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] as usize } else { 0 };

            result.push(TABLE[(b0 >> 2) & 0x3F] as char);
            result.push(TABLE[((b0 << 4) | (b1 >> 4)) & 0x3F] as char);
            if chunk.len() > 1 {
                result.push(TABLE[((b1 << 2) | (b2 >> 6)) & 0x3F] as char);
            }
            if chunk.len() > 2 {
                result.push(TABLE[b2 & 0x3F] as char);
            }
        }
        result
    }
}
