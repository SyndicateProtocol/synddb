# SyndDB Bridge Smart Contracts

## Overview

This directory contains the smart contracts for the SyndDB Bridge system, which enables secure asset bridging between SyndDB's high-performance database layer and the blockchain.

## Contract Architecture

### Core Contracts

#### Bridge.sol
The main bridge contract handling:
- **Deposits**: Users lock tokens to use in SyndDB
- **Withdrawals**: Process withdrawals with m-of-n validator signatures
- **Batch Settlements**: Handle complex orderbook state transitions
- **Circuit Breakers**: Security limits and emergency controls
- **Validator Management**: Registry of TEE validators

#### TEEAttestationVerifier.sol
Handles dual attestation verification:
- **SP1 Proof Verification**: Zero-knowledge proofs of TEE state
- **Lit Protocol Verification**: Decentralized attestation verification
- **Measurement Validation**: Verify TEE environments match expected configurations

### Interfaces

- **IBridge.sol**: Interface for the main bridge contract
- **ITEEAttestationVerifier.sol**: Interface for attestation verification

### Libraries

- **SignatureUtils.sol**: Multi-signature verification utilities
- **MerkleUtils.sol**: Merkle tree operations for batch settlements
- **CircuitBreaker.sol**: Rate limiting and circuit breaker patterns

## Key Features

### Security Features
- **M-of-N Multisig**: Requires multiple validator signatures for withdrawals
- **TEE Attestation**: Validators must prove they run in secure enclaves
- **Circuit Breakers**: Daily/hourly withdrawal limits
- **Emergency Pause**: Admin-controlled emergency stop
- **Nonce Management**: Prevents replay attacks

### Gas Optimizations
- **Batch Processing**: Multiple operations in single transaction
- **Storage Packing**: Efficient struct layouts
- **Merkle Proofs**: Compress large state updates

### Validator System
- **Dynamic Set**: Validators can be added/removed
- **Dual Attestation**: SP1 + Lit Protocol verification
- **Key Rotation**: Support for updating validator keys

## Deployment

### Prerequisites
```bash
npm install @openzeppelin/contracts
npm install hardhat
```

### Deploy Steps
1. Deploy TEEAttestationVerifier with SP1 verifier address
2. Deploy Bridge with initial validators and sequencer
3. Configure expected TEE measurements
4. Set circuit breaker limits
5. Enable deposits and withdrawals

### Configuration Parameters
```solidity
// Initial deployment parameters
address sequencer = 0x...;           // SyndDB sequencer address
address[] validators = [...];        // Initial validator set
uint256 requiredSignatures = 3;      // M in M-of-N
uint256 depositFeeBps = 10;          // 0.1%
uint256 withdrawalFeeBps = 30;       // 0.3%
uint256 dailyLimit = 10_000_000e18;  // $10M daily limit
```

## Testing

### Unit Tests
```bash
npx hardhat test test/Bridge.test.js
npx hardhat test test/TEEAttestationVerifier.test.js
```

### Integration Tests
```bash
npx hardhat test test/integration/FullFlow.test.js
```

### Gas Benchmarks
```bash
npx hardhat test test/gas/GasBenchmark.test.js
```

## Security Considerations

### Critical Invariants
1. Total deposits >= Total withdrawals + fees
2. Nonces must be strictly increasing
3. Validators cannot sign same nonce twice
4. Circuit breakers must reset daily

### Audit Focus Areas
- Signature verification logic
- Nonce management
- Fee calculations
- Merkle proof verification
- Circuit breaker logic

## Integration Guide

### For SyndDB Sequencer
```javascript
// Monitor deposit events
bridge.on('Deposit', async (depositor, token, amount, syndDbAccountId) => {
    // Credit user in database
    await creditUser(syndDbAccountId, token, amount);
});

// Submit withdrawal
const message = {
    nonce: currentNonce++,
    recipient: userAddress,
    token: tokenAddress,
    amount: withdrawalAmount,
    deadline: Math.floor(Date.now() / 1000) + 3600
};
const signature = await sequencer.signMessage(message);
```

### For Validators
```javascript
// Verify and sign withdrawal
const isValid = await verifyDatabaseState(message);
if (isValid) {
    const signature = await validator.signMessage(message);
    await submitSignature(message.nonce, signature);
}
```

### For Relayers
```javascript
// Submit withdrawal with collected signatures
await bridge.processWithdrawal(
    message.nonce,
    message.recipient,
    message.token,
    message.amount,
    message.deadline,
    sequencerSignature,
    validatorSignatures
);
```

## Mainnet Deployment Checklist

- [ ] Complete code audit by 2 firms
- [ ] Formal verification of critical functions
- [ ] Deploy to testnet and run for 30 days
- [ ] Set conservative initial limits
- [ ] Configure monitoring and alerts
- [ ] Prepare incident response procedures
- [ ] Document admin key management
- [ ] Create upgrade plan

## License

MIT