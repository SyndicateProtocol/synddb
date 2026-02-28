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
    storage::{StorageAddress, StorageBool, StorageMap, StorageU64},
};

/// The ecrecover precompile address (0x01)
const ECRECOVER: Address = Address::with_last_byte(0x01);

sol! {
    /// Public values structure for GCP Confidential Space attestations.
    /// Must match the Solidity `PublicValuesStruct` in `RiscZeroAttestationVerifier`.
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

    event TrustedAttestorAdded(address indexed attestor);
    event TrustedAttestorRemoved(address indexed attestor);
    event TrustedJwkHashAdded(bytes32 indexed jwk_hash);
    event TrustedJwkHashRemoved(bytes32 indexed jwk_hash);
    event AllowedImageDigestHashAdded(bytes32 indexed digest_hash);
    event AllowedImageDigestHashRemoved(bytes32 indexed digest_hash);
    event TrustedImageSignerAdded(address indexed signer);
    event TrustedImageSignerRemoved(address indexed signer);
    event OwnershipTransferred(address indexed previous_owner, address indexed new_owner);

    error NotOwner(address caller);
    error AlreadyInitialized();
    error NotInitialized();
    error UntrustedAttestor(address attestor);
    error UntrustedJwkHash(bytes32 jwk_hash);
    error ValidityWindowNotStarted(uint64 start, uint64 current);
    error ValidityWindowExpired(uint64 end, uint64 current);
    error SecureBootRequired();
    error DebugModeNotAllowed();
    error UntrustedImageDigest(bytes32 digest_hash);
    error UntrustedImageSigner(address signer);
    error ImageSignatureInvalid();
    error EcrecoverFailed();
    error InvalidProofLength();
    error InvalidPublicValues();
}

#[derive(SolidityError)]
pub enum VerifierError {
    NotOwner(NotOwner),
    AlreadyInitialized(AlreadyInitialized),
    NotInitialized(NotInitialized),
    UntrustedAttestor(UntrustedAttestor),
    UntrustedJwkHash(UntrustedJwkHash),
    ValidityWindowNotStarted(ValidityWindowNotStarted),
    ValidityWindowExpired(ValidityWindowExpired),
    SecureBootRequired(SecureBootRequired),
    DebugModeNotAllowed(DebugModeNotAllowed),
    UntrustedImageDigest(UntrustedImageDigest),
    UntrustedImageSigner(UntrustedImageSigner),
    ImageSignatureInvalid(ImageSignatureInvalid),
    EcrecoverFailed(EcrecoverFailed),
    InvalidProofLength(InvalidProofLength),
    InvalidPublicValues(InvalidPublicValues),
}

/// Stylus attestation verifier for `SyndDB` TEE key registration.
///
/// This contract provides an alternative to the `RiscZeroAttestationVerifier` by
/// verifying attestation claims directly on-chain using Arbitrum Stylus (WASM).
/// Instead of verifying a RISC Zero Groth16 ZK proof, it verifies an ECDSA
/// signature from a trusted attestor who has validated the GCP Confidential Space
/// JWT off-chain.
///
/// The contract implements the same `IAttestationVerifier` interface as the
/// Solidity verifier, making it a drop-in replacement via
/// `TeeKeyManager.updateAttestationVerifier()`.
///
/// Verification flow:
/// 1. A trusted attestor verifies the GCP Confidential Space JWT off-chain
/// 2. The attestor ABI-encodes the extracted claims into `PublicValuesStruct`
/// 3. The attestor signs `keccak256(publicValues)` with its ECDSA key
/// 4. This contract recovers the attestor address via ecrecover
/// 5. If the attestor is trusted, validates all attestation claims
/// 6. Returns the TEE signing key address from the attestation
#[storage]
#[entrypoint]
pub struct StylusAttestationVerifier {
    /// Contract owner (can manage trusted attestors and configuration)
    owner: StorageAddress,
    /// Whether the contract has been initialized
    initialized: StorageBool,
    /// Grace period in seconds after token expiration
    expiration_tolerance: StorageU64,
    /// Trusted attestor addresses that can sign attestation claims
    trusted_attestors: StorageMap<Address, StorageBool>,
    /// Trusted JWK key hashes (keccak256 of JWK key ID)
    trusted_jwk_hashes: StorageMap<FixedBytes<32>, StorageBool>,
    /// Allowed container image digest hashes
    allowed_image_digest_hashes: StorageMap<FixedBytes<32>, StorageBool>,
    /// Trusted image signer addresses
    trusted_image_signers: StorageMap<Address, StorageBool>,
}

#[public]
impl StylusAttestationVerifier {
    /// Initializes the contract (replaces constructor for Stylus contracts).
    ///
    /// Must be called once after deployment. Sets the caller as the owner.
    pub fn initialize(&mut self, expiration_tolerance: u64) -> Result<(), VerifierError> {
        if self.initialized.get() {
            return Err(VerifierError::AlreadyInitialized(AlreadyInitialized {}));
        }
        self.owner.set(self.vm().msg_sender());
        self.expiration_tolerance.set(U64::from(expiration_tolerance));
        self.initialized.set(true);
        Ok(())
    }

    /// Verifies an attestation proof and returns the TEE signing key.
    ///
    /// This function is ABI-compatible with `IAttestationVerifier.verifyAttestationProof`.
    /// The Solidity function selector for `verifyAttestationProof(bytes,bytes)` matches
    /// the Stylus-generated selector for this method.
    ///
    /// # Arguments
    /// * `public_values` - ABI-encoded `PublicValuesStruct` containing attestation claims
    /// * `proof_bytes` - 65-byte ECDSA signature (r || s || v) from a trusted attestor
    ///   over `keccak256(public_values)`
    ///
    /// # Returns
    /// The TEE signing key address extracted from the attestation
    pub fn verify_attestation_proof(
        &self,
        public_values: Bytes,
        proof_bytes: Bytes,
    ) -> Result<Address, VerifierError> {
        // Validate proof length (65 bytes: r[32] || s[32] || v[1])
        if proof_bytes.len() != 65 {
            return Err(VerifierError::InvalidProofLength(InvalidProofLength {}));
        }

        // Recover attestor address from signature over keccak256(publicValues)
        let public_values_hash: FixedBytes<32> = self.vm().native_keccak256(&public_values).into();
        let r = FixedBytes::<32>::from_slice(&proof_bytes[0..32]);
        let s = FixedBytes::<32>::from_slice(&proof_bytes[32..64]);
        let v = proof_bytes[64];

        let attestor = self.ecrecover_call(public_values_hash, v, r, s)?;

        // Verify the attestor is trusted
        if !self.trusted_attestors.get(attestor) {
            return Err(VerifierError::UntrustedAttestor(UntrustedAttestor {
                attestor,
            }));
        }

        // Decode PublicValuesStruct from ABI-encoded bytes
        let values = PublicValuesStruct::abi_decode(&public_values)
            .map_err(|_| VerifierError::InvalidPublicValues(InvalidPublicValues {}))?;

        // Validate JWK hash is trusted
        if !self.trusted_jwk_hashes.get(values.jwk_key_hash) {
            return Err(VerifierError::UntrustedJwkHash(UntrustedJwkHash {
                jwk_hash: values.jwk_key_hash,
            }));
        }

        // Validate validity window
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

        // Validate secure boot
        if !values.secboot {
            return Err(VerifierError::SecureBootRequired(SecureBootRequired {}));
        }

        // Validate debug mode is disabled
        if !values.dbgstat_disabled {
            return Err(VerifierError::DebugModeNotAllowed(DebugModeNotAllowed {}));
        }

        // Validate image digest hash
        if !self.allowed_image_digest_hashes.get(values.image_digest_hash) {
            return Err(VerifierError::UntrustedImageDigest(UntrustedImageDigest {
                digest_hash: values.image_digest_hash,
            }));
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

    /// Adds a trusted attestor address.
    pub fn add_trusted_attestor(&mut self, attestor: Address) -> Result<(), VerifierError> {
        self.only_owner()?;
        self.trusted_attestors.setter(attestor).set(true);
        self.vm().log(TrustedAttestorAdded { attestor });
        Ok(())
    }

    /// Removes a trusted attestor address.
    pub fn remove_trusted_attestor(&mut self, attestor: Address) -> Result<(), VerifierError> {
        self.only_owner()?;
        self.trusted_attestors.setter(attestor).set(false);
        self.vm().log(TrustedAttestorRemoved { attestor });
        Ok(())
    }

    /// Adds a trusted JWK hash.
    pub fn add_trusted_jwk_hash(&mut self, jwk_hash: FixedBytes<32>) -> Result<(), VerifierError> {
        self.only_owner()?;
        self.trusted_jwk_hashes.setter(jwk_hash).set(true);
        self.vm().log(TrustedJwkHashAdded { jwk_hash });
        Ok(())
    }

    /// Removes a trusted JWK hash.
    pub fn remove_trusted_jwk_hash(
        &mut self,
        jwk_hash: FixedBytes<32>,
    ) -> Result<(), VerifierError> {
        self.only_owner()?;
        self.trusted_jwk_hashes.setter(jwk_hash).set(false);
        self.vm().log(TrustedJwkHashRemoved { jwk_hash });
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

    /// Transfers ownership of the contract.
    pub fn transfer_ownership(&mut self, new_owner: Address) -> Result<(), VerifierError> {
        self.only_owner()?;
        let previous_owner = self.owner.get();
        self.owner.set(new_owner);
        self.vm().log(OwnershipTransferred {
            previous_owner,
            new_owner,
        });
        Ok(())
    }

    // -- View functions --

    /// Returns the contract owner.
    pub fn owner(&self) -> Address {
        self.owner.get()
    }

    /// Returns whether the contract is initialized.
    pub fn is_initialized(&self) -> bool {
        self.initialized.get()
    }

    /// Returns the expiration tolerance in seconds.
    pub fn expiration_tolerance(&self) -> U64 {
        self.expiration_tolerance.get()
    }

    /// Returns whether an attestor is trusted.
    pub fn is_attestor_trusted(&self, attestor: Address) -> bool {
        self.trusted_attestors.get(attestor)
    }

    /// Returns whether a JWK hash is trusted.
    pub fn is_jwk_hash_trusted(&self, jwk_hash: FixedBytes<32>) -> bool {
        self.trusted_jwk_hashes.get(jwk_hash)
    }

    /// Returns whether an image digest hash is allowed.
    pub fn is_image_digest_hash_allowed(&self, digest_hash: FixedBytes<32>) -> bool {
        self.allowed_image_digest_hashes.get(digest_hash)
    }

    /// Returns whether an image signer is trusted.
    pub fn is_image_signer_trusted(&self, signer: Address) -> bool {
        self.trusted_image_signers.get(signer)
    }
}

impl StylusAttestationVerifier {
    /// Verifies that the caller is the contract owner.
    fn only_owner(&self) -> Result<(), VerifierError> {
        let sender = self.vm().msg_sender();
        if sender != self.owner.get() {
            return Err(VerifierError::NotOwner(NotOwner { caller: sender }));
        }
        Ok(())
    }

    /// Calls the ecrecover precompile to recover an address from a signature.
    ///
    /// The ecrecover precompile at address 0x01 accepts 128 bytes:
    /// - bytes 0-31: message hash
    /// - bytes 32-63: v (recovery ID, left-padded to 32 bytes)
    /// - bytes 64-95: r component
    /// - bytes 96-127: s component
    fn ecrecover_call(
        &self,
        hash: FixedBytes<32>,
        v: u8,
        r: FixedBytes<32>,
        s: FixedBytes<32>,
    ) -> Result<Address, VerifierError> {
        // Build raw 128-byte input for ecrecover precompile
        let mut input = Vec::with_capacity(128);
        input.extend_from_slice(hash.as_slice());
        let mut v_padded = [0u8; 32];
        v_padded[31] = v;
        input.extend_from_slice(&v_padded);
        input.extend_from_slice(r.as_slice());
        input.extend_from_slice(s.as_slice());

        match call::static_call(self.vm(), Call::new(), ECRECOVER, &input) {
            Ok(result) => {
                if result.len() < 32 {
                    return Err(VerifierError::EcrecoverFailed(EcrecoverFailed {}));
                }
                // Address is right-aligned in the 32-byte return value (bytes 12..32)
                Ok(Address::from_slice(&result[12..32]))
            }
            Err(_) => Err(VerifierError::EcrecoverFailed(EcrecoverFailed {})),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_ecrecover_input_format() {
        // Verify the raw 128-byte input format is correct
        let hash = FixedBytes::<32>::ZERO;
        let v: u8 = 27;
        let r = FixedBytes::<32>::ZERO;
        let s = FixedBytes::<32>::ZERO;

        let mut input = Vec::with_capacity(128);
        input.extend_from_slice(hash.as_slice());
        let mut v_padded = [0u8; 32];
        v_padded[31] = v;
        input.extend_from_slice(&v_padded);
        input.extend_from_slice(r.as_slice());
        input.extend_from_slice(s.as_slice());

        assert_eq!(input.len(), 128);
        // v should be at byte 63 (the last byte of the second 32-byte chunk)
        assert_eq!(input[63], 27);
    }
}
