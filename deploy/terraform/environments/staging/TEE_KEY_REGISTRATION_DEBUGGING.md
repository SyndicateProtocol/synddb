# TEE Key Registration: Debugging Guide

This document captures lessons learned from debugging TEE key registration failures and provides guidance for future troubleshooting.

## Architecture Overview

A successful TEE key registration requires coordination between:

```
Sequencer/Validator (Confidential Space VM)
    │
    ├── Generates ephemeral signing key
    ├── Fetches attestation token from TEE server
    ├── Requests RISC Zero proof from proof-service
    ├── Signs EIP-712 key registration message
    │
    ▼
Relayer (Cloud Run)
    │
    ├── Validates attestation locally
    ├── Calls Bridge.registerKeyWithSignature()
    │
    ▼
Bridge Contract
    │
    ├── Calls TeeKeyManager.addKeyWithSignature()
    │
    ▼
TeeKeyManager Contract
    │
    ├── Calls AttestationVerifier.verifyAndRegisterKey()
    │
    ▼
AttestationVerifier Contract
    │
    ├── Verifies RISC Zero proof (via RiscZeroGroth16Verifier)
    ├── Verifies image signature (ecrecover)
    ├── Verifies image digest hash matches expected
    ├── Verifies validity window is current
    │
    ▼
Key Registered On-Chain
    │
    ▼
Bootstrap Verifies via TeeKeyManager.isKeyValid()
    │
    ▼
Service Starts
```

If any link in this chain fails, you get a generic "execution reverted" error.

## Common Failure Modes

### 1. Contract Interface Mismatch

**Symptoms:**
- Key registration transaction reverts
- Error: "execution reverted" with no additional context

**Cause:**
Bridge and TeeKeyManager deployed with incompatible interfaces. This happened during the KeyType refactoring where separate `registerSequencerKeyWithSignature` / `registerValidatorKeyWithSignature` functions were unified into `registerKeyWithSignature(KeyType, ...)`.

**Diagnosis:**
```bash
# Check what functions exist on each contract
cast call $BRIDGE "teeKeyManager()(address)" --rpc-url $RPC_URL
cast call $TEE_KEY_MANAGER "isKeyValid(uint8,address)(bool)" 0 $ADDRESS --rpc-url $RPC_URL
```

**Fix:**
Redeploy both Bridge and TeeKeyManager together to ensure interface compatibility.

### 2. Rust Code Using Old Interface

**Symptoms:**
- Key registration transaction SUCCEEDS on-chain
- But bootstrap reports "Key verification failed: execution reverted"
- `isKeyValid` returns true when called manually

**Cause:**
The Rust code is calling a function that doesn't exist on the contract. Check these files:
- `crates/synddb-relayer/src/submitter.rs` - Relayer's Bridge calls
- `crates/synddb-relayer/src/handlers.rs` - Relayer's verification calls
- `crates/synddb-bootstrap/src/submitter.rs` - Bootstrap's verification calls

**Diagnosis:**
```bash
# Search for old interface names in Rust code
grep -r "isSequencerKeyValid\|isValidatorKeyValid" --include="*.rs" crates/
grep -r "registerSequencerKey\|registerValidatorKey" --include="*.rs" crates/
```

**Fix:**
Update the `sol!` macro interfaces in the Rust code to match the deployed contracts.

### 3. Image Digest Mismatch

**Symptoms:**
- Attestation verification fails
- Proof is valid but registration reverts

**Cause:**
The running VM has a different image digest than what's registered in AttestationVerifier.

**Diagnosis:**
```bash
# Get expected digest hash from AttestationVerifier
cast call $ATTESTATION_VERIFIER "expectedImageDigestHash()(bytes32)" --rpc-url $RPC_URL

# Compute hash from current sequencer + validator digests
# (The hash is keccak256(sequencer_digest || validator_digest))
```

**Fix:**
Either:
1. Update AttestationVerifier with new expected hash (via `setExpectedImageDigestHash`)
2. Redeploy VMs with the expected image digests

### 4. RISC Zero Image ID Mismatch

**Symptoms:**
- Proof verification fails in AttestationVerifier
- Error in RISC Zero verifier

**Cause:**
The proof was generated with a different guest program than what's registered on-chain.

**Diagnosis:**
```bash
# Get expected image ID from Bridge
cast call $BRIDGE "attestationVerifier()(address)" --rpc-url $RPC_URL
cast call $ATTESTATION_VERIFIER "imageId()(bytes32)" --rpc-url $RPC_URL

# Get image ID from proof-service container
oras discover $PROOF_SERVICE_IMAGE
# Look for application/vnd.syndicate.risc0-image-id.v1+json artifact
```

**Fix:**
Update the RISC Zero image ID on-chain via `setImageId`.

### 5. Validity Window Expired

**Symptoms:**
- Registration fails with validity check error
- Works when retried quickly after VM restart

**Cause:**
Attestation tokens have a 1-hour validity window. If proof generation takes too long or there's a delay, the token expires.

**Diagnosis:**
Check the `eat_nonce` timestamps in the relayer debug logs.

**Fix:**
Ensure proof generation completes within the validity window. Consider reducing proof complexity or using faster hardware.

## Debugging Checklist

When key registration fails:

1. **Enable debug logging on relayer**
   ```hcl
   # In main.tf
   module "relayer" {
     rust_log = "debug"
   }
   ```

2. **Check relayer logs for the full proof data**
   ```bash
   gcloud logging read 'resource.type="cloud_run_revision" AND labels.service_name=~"relayer"' \
     --project=$PROJECT --limit=50 --format='value(textPayload,jsonPayload)'
   ```

3. **Verify contract addresses match**
   ```bash
   # Bridge should point to correct TeeKeyManager
   cast call $BRIDGE "teeKeyManager()(address)" --rpc-url $RPC_URL

   # TeeKeyManager should point to correct AttestationVerifier
   cast call $TEE_KEY_MANAGER "attestationVerifier()(address)" --rpc-url $RPC_URL
   ```

4. **Verify on-chain configuration**
   ```bash
   # Check expected image digest hash
   cast call $ATTESTATION_VERIFIER "expectedImageDigestHash()(bytes32)" --rpc-url $RPC_URL

   # Check RISC Zero image ID
   cast call $ATTESTATION_VERIFIER "imageId()(bytes32)" --rpc-url $RPC_URL

   # Check image signer
   cast call $ATTESTATION_VERIFIER "imageSigner()(address)" --rpc-url $RPC_URL
   ```

5. **Test attestation verification directly**
   ```bash
   # Call verifyAttestation with the proof data from logs
   cast call $ATTESTATION_VERIFIER \
     "verifyAttestation(bytes,bytes)(bool)" \
     $PUBLIC_VALUES $PROOF_BYTES \
     --rpc-url $RPC_URL
   ```

6. **Verify key registration succeeded**
   ```bash
   # KeyType: 0 = Sequencer, 1 = Validator
   cast call $TEE_KEY_MANAGER "isKeyValid(uint8,address)(bool)" 0 $KEY_ADDRESS --rpc-url $RPC_URL
   ```

## Configuration Locations

| Config | Location | Notes |
|--------|----------|-------|
| Contract addresses | `terraform.tfvars` | Bridge, TeeKeyManager set here |
| Image digests | `terraform.tfvars` | Must match actual deployed images |
| Image signatures | OCI artifacts | Attached to images in Artifact Registry |
| RISC Zero image ID | OCI artifacts | Attached to proof-service image |
| Expected digest hash | AttestationVerifier | Set via Terraform null_resource |
| Expected RISC0 ID | Bridge | Set via Terraform null_resource |

## Contract Upgrade Runbook

When upgrading contracts with interface changes:

1. **Update Solidity contracts** in `contracts/src/`
2. **Run contract tests** to verify compatibility
3. **Update Rust interfaces** in all three locations:
   - `crates/synddb-relayer/src/submitter.rs`
   - `crates/synddb-relayer/src/handlers.rs`
   - `crates/synddb-bootstrap/src/submitter.rs`
4. **Deploy contracts together** - TeeKeyManager and Bridge must be compatible
5. **Update terraform.tfvars** with new contract addresses
6. **Run terraform apply** to update VMs and on-chain config
7. **Verify key registration** works end-to-end

## Future Improvements

See inline TODOs for automation opportunities:

- [ ] CI check for interface consistency between Rust and Solidity
- [ ] Contract compatibility verification in Terraform
- [ ] Bootstrap health endpoint with detailed status
- [ ] Contract deployment lockfile for version tracking
- [ ] Automated interface grep checks in lint

## Related Files

- `crates/synddb-bootstrap/` - Bootstrap state machine
- `crates/synddb-relayer/` - Relayer service
- `contracts/src/attestation/` - Attestation contracts
- `deploy/terraform/environments/staging/` - Staging deployment config
