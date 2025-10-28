# PLAN_BRIDGE.md - SyndDB Bridge Smart Contract Architecture

## Executive Summary
The Bridge.sol contract serves as the settlement layer interface for SyndDB, enabling secure asset bridging between the high-performance database layer and the blockchain. It supports message passing to/from the SyndDB database for diverse use cases including onchain orderbooks for DeFi, onchain telemetry for DePIN, onchain assets for gaming, onchain social data, and more. The bridge requires multi-signature validation from TEE validators and allows permissionless relaying of signed messages.


## Core Architecture Principles

### 1. Asymmetric Trust Model
- **Deposits**: Trustless (any user can deposit)
- **Withdrawals**: Validators can only sign messages that are first signed by the sequencer - they validate sequencer messages, not arbitrary withdrawals
- **State Updates**: Batched updates for general message passing (not limited to orderbooks)

### 2. Permissioned TEE Relaying
- **TEE-based relayer**: Relayer runs in TEE environment (similar to validators)
- **Permissioned role**: Trusted relayer eliminates front-running and withholding attacks
- **Batching optimization**: Relayer handles batching of multiple messages for gas efficiency
- **Guaranteed submission**: No censorship or selective message withholding
- **Attestation required**: Relayer must provide TEE attestation like validators
- Messages include nonces to prevent replay attacks
- Batching happens at relayer level, not validator level - validators simply sign individual messages


### 3. Complex State Transitions
- Not limited to 1:1 token swaps
- Supports various settlement types with multiple balance updates (orderbooks, gaming, DePIN rewards, etc.)
- Handles partial operations and complex state changes across different use cases


## Key Components

### 1. Deposit System
- **Direct deposits**: Users lock tokens in bridge
- **Deposit receipts**: Emitted for sequencer to credit in SyndDB
- **Multi-token support**: ETH, ERC-20, potentially ERC-721/1155
- **Deposit limits**: Configurable per-user and global circuit breakers for risk reduction during early launches
- **Lock vs Bridge Model**:
  - Most use cases are short-term locks (unlocked upon settlement)
  - Some use cases are long-term bridges (tokens stay in platform for extended periods)
  - System supports both models seamlessly


### 2. Validator & Relayer Registry
- **TEE attestation verification**: Both validators and relayers prove TEE environment
- **Dual attestation**: Combines TEE + zkVM security through SP1 proofs and Lit Protocol verification
- **Two-phase onboarding**:
  - Validators/relayers can permissionlessly propose themselves by submitting valid SP1 proof and Lit Protocol attestation
  - Actual addition to validator/relayer set remains permissioned (governance controlled)
- **Dynamic sets**: Validators and relayers can be added/removed by governance
- **Key rotation**: Support for validator and relayer key updates
- **Role separation**: Validators attest to state, relayers submit transactions


### 3. Message Processing & Contract Separation

#### Bridge.sol - Token Movement Focus
- **Purpose**: Handles tokens in → tokens out, not state management
- **Sequencer messages**: Signed withdrawal/settlement messages
- **Validator confirmations**: m-of-n signatures required to validate sequencer messages
- **Message types**:
  - Withdrawals (single user, single token)
  - Batch settlements (multiple users, multiple tokens)
  - Emergency actions (pause, circuit breaker triggers)

#### Chain.sol (Separate Contract) - State Management & Version Control
Following OP Stack's separation pattern:
- **State publication**: Handles diffs/snapshots from sequencer
- **State verification**: Validators attest to state correctness
- **State availability**: Makes state available for Bridge.sol validation
- **Withdrawal gating**: Bridge.sol only processes withdrawals after Chain.sol confirms state
- **WASM version pinning**:
  - Stores current WASM version hash and IPFS/Arweave CID
  - TEEs bootstrap from pinned version without full replacement
  - Enables coordinated upgrades across sequencer/validators
  - Version changes require governance approval

This separation mirrors OP Stack's approach where:
- OptimismPortal handles deposits/withdrawals
- L2OutputOracle manages state roots
- Clean interfaces between contracts for modularity


### 4. Settlement Engine
- **Batch processing**: Multiple operations in single transaction
- **Tunable batching**:
  - Minimum latency mode: Batch time = block time (or Flashblocks for chains like Base)
  - Gas optimization mode: Larger batches for cost efficiency
  - Clear tradeoff: Lower latency vs. higher gas costs per operation
- **Atomic execution**: All-or-nothing settlement
- **Gas optimization**: Efficient storage patterns and batch operations
- **Slippage protection**: Max deviation from expected state


### 5. Security Mechanisms
- **Circuit breakers**: Daily/hourly withdrawal limits
- **Dual-layer protection**:
  - Onchain circuit breakers (enforced by smart contracts)
  - Offchain circuit breakers (validators can refuse to sign risky withdrawals)
- **Time delays**: Optional delay for large withdrawals
- **Pause functionality**: Emergency stop mechanism
- **Rate limiting**: Per-user withdrawal frequency limits
- **Merkle proofs**: For large batch settlements


## Data Structures

```solidity
struct Validator {
    address publicKey;
    bytes32 attestationHash;
    bool isActive;
    uint256 addedAt;
    bytes32 wasmVersionHash; // Version this validator is running
}

struct DepositRecord {
    address depositor;
    address token;
    uint256 amount;
    uint256 blockNumber;
    bytes32 syndDbAccountId;
}

struct WithdrawalMessage {
    uint256 nonce;
    address recipient;
    address token;
    uint256 amount;
    uint256 deadline;
    bytes sequencerSignature;
}

struct BatchSettlement {
    uint256 nonce;
    bytes32 stateRoot;
    BalanceUpdate[] updates;
    uint256 deadline;
    bytes sequencerSignature;
}

struct BalanceUpdate {
    address account;
    address token;
    int256 delta; // Can be negative for debits
}

// Chain.sol specific structures
struct WASMVersion {
    bytes32 versionHash;
    string ipfsCID;      // IPFS CID of WASM binary
    string arweaveTxId;  // Arweave transaction ID (backup)
    uint256 activationBlock;
    bool isActive;
}

struct StateCommitment {
    bytes32 stateRoot;
    bytes32 wasmVersionHash; // Version that produced this state
    uint256 sequencerVersion;
    uint256 timestamp;
}
```

## Process Flows

### Deposit Flow
1. User approves tokens to Bridge contract
2. User calls `deposit(token, amount, syndDbAccount)`
3. Bridge locks tokens
4. Bridge emits `Deposit` event
5. Sequencer detects event and credits user in SyndDB
6. User can trade on SyndDB immediately

### Withdrawal Flow
1. User requests withdrawal in SyndDB
2. Sequencer validates and signs withdrawal message
3. Validators verify state and add signatures to the sequencer's message
4. **TEE Relayer** collects m-of-n validator signatures
5. **TEE Relayer** submits batch to bridge contract with guaranteed ordering
6. Bridge validates all signatures and nonce ordering:
   - **Global nonce**: Ensures sequential message processing
   - **Per-user nonce**: Prevents user-specific replay attacks
   - **Re-org protection**: Nonces map to transaction data (not just increments)
   - **Validator safety**: Validators halt signing if nonces unexpectedly change (re-org detection)
7. Bridge transfers tokens to recipient
8. Bridge emits `Withdrawal` event

Note: The deposit/withdrawal naming is preferred over lock/unlock since there's no 1:1 relationship between tokens in and out (due to fees, settlement netting, etc.)


### Batch Settlement Flow (State-Dependent)
1. **State Publication Phase (Chain.sol)**:
   - Sequencer publishes state diffs/snapshots to Chain.sol
   - State commitment includes WASM version hash that produced it
   - Validators verify state correctness and WASM version match
   - State becomes available onchain once validated

2. **Settlement Phase (Bridge.sol)**:
   - Sequencer computes net position changes based on published state
   - Sequencer creates merkle tree of balance updates
   - Sequencer signs batch settlement message referencing Chain.sol state
   - Validators verify state availability in Chain.sol before signing
   - **TEE Relayer** submits batch only after state is confirmed in Chain.sol
   - Bridge processes updates atomically
   - Net token movements executed on-chain

Note: Withdrawals are gated on state availability - Bridge.sol checks Chain.sol to ensure the required state has been published and validated before processing any withdrawals.

### WASM Version Management Flow
1. **Version Deployment**:
   - New WASM binary uploaded to IPFS and Arweave
   - Governance proposal to update version in Chain.sol
   - Version hash and storage pointers recorded onchain

2. **TEE Bootstrap Process**:
   - TEE reads current version from Chain.sol
   - Downloads WASM from IPFS/Arweave using pinned CID
   - Verifies hash matches onchain record
   - Loads and executes WASM without TEE replacement

3. **Coordinated Upgrade**:
   - Governance sets activation block for new version
   - All TEEs monitor for version change
   - At activation block, TEEs hot-swap to new WASM
   - Old state remains valid, new state uses new version

4. **Version Verification**:
   - Each state commitment includes WASM version hash
   - Validators only sign states from matching versions
   - Bridge.sol can verify version consistency

## Security Considerations

### Attack Vectors & Mitigations

#### 1. Sequencer Compromise
- **Mitigation**: m-of-n validator requirement
- **Mitigation**: Daily withdrawal limits
- **Mitigation**: Time delays for large amounts

#### 2. Validator Collusion
- **Mitigation**: TEE attestation requirements
- **Mitigation**: Dual attestation (SP1 + Lit)
- **Mitigation**: Validator rotation mechanism
- **Mitigation**: Economic stakes/slashing

#### 3. Replay Attacks
- **Mitigation**: Strict nonce ordering
- **Mitigation**: Message deadlines
- **Mitigation**: Chain ID in signatures (especially for cross-chain deposits)


#### 4. Transaction Withholding/Censorship
- **Eliminated by design**: Permissioned TEE relayer guarantees submission
- **No selective processing**: Relayer must submit all valid messages
- **Attestation enforcement**: TEE environment prevents malicious behavior

#### 5. WASM Version Attacks
- **Mitigation**: Version hash verification in TEE bootstrap
- **Mitigation**: Dual storage (IPFS + Arweave) for availability
- **Mitigation**: Governance-controlled version updates
- **Mitigation**: State includes version hash for auditability

#### 6. Gas Griefing
- **Mitigation**: TEE relayer optimizes gas usage
- **Mitigation**: Batch processing limits
- **Mitigation**: Storage optimization

## Implementation Phases

### Phase 1: Core Bridge (Week 1-2)
- Deposit/withdrawal primitives
- Basic validator registry
- Simple signature verification
- ERC-20 support only

### Phase 2: Validator Integration (Week 3)
- TEE attestation verification
- SP1 proof verification
- Lit Protocol integration
- Validator rotation logic
- WASM version pinning in Chain.sol

### Phase 3: Batch Settlement (Week 4)
- Merkle tree verification
- Batch processing engine
- Gas optimizations
- Atomic multi-token operations

### Phase 4: Security Features (Week 5)
- Circuit breakers implementation
- Time delays and rate limiting
- Emergency pause system
- Monitoring events

### Phase 5: Advanced Features (Week 6)
- ETH native support
- NFT bridging (ERC-721/1155)
- Cross-chain messaging
- Governance integration
- WASM hot-swapping mechanism
- Version migration tooling

## Gas Optimization Strategies

### 1. Storage Packing
- Pack structs efficiently
- Use bytes32 for IDs instead of strings
- Minimize storage writes

### 2. Batch Operations
- Process multiple withdrawals together
- Aggregate signature verification
- Merkle proofs for large batches

### 3. Call Data Optimization
- Compress message data
- Use efficient encoding
- Minimize signature sizes

## Monitoring & Analytics

### On-chain Events
- `Deposit(user, token, amount, syndDbAccount)`
- `Withdrawal(user, token, amount, nonce)`
- `BatchSettlement(stateRoot, updateCount)`
- `ValidatorAdded(address, attestation)`
- `ValidatorRemoved(address, reason)`
- `CircuitBreakerTriggered(reason, duration)`
- `WASMVersionUpdated(oldHash, newHash, ipfsCID, activationBlock)`
- `StateCommitted(stateRoot, wasmVersionHash, sequencerVersion)`

### Off-chain Monitoring
- TVL tracking per token
- Withdrawal queue depth
- Validator participation rates
- Gas costs and relayer profitability
- Settlement frequency and size

## Upgrade Path

### 1. Proxy Pattern
- Use OpenZeppelin upgradeable contracts
- Transparent proxy for admin functions
- Time-locked upgrades

### 2. Migration Strategy
- Pause old bridge
- Allow withdrawal-only mode
- Deploy new bridge
- Move liquidity atomically

## Integration Requirements

### For SyndDB Sequencer
- Monitor deposit events
- Generate withdrawal messages
- Compute batch settlements
- Sign messages with correct format

### For Validators
- Verify database state
- Generate attestation proofs
- Sign messages when valid
- Monitor for malicious activity

### For TEE Relayer
- Run in TEE environment with attestation
- Monitor for signed messages from validators
- Batch messages efficiently for gas optimization
- Submit all valid messages (no censorship)
- Handle revert scenarios and retry logic
- Maintain submission queue integrity

## Testing Requirements

### 1. Unit Tests
- Signature verification
- Merkle proof validation
- Circuit breaker logic
- Nonce management

### 2. Integration Tests
- Full deposit/withdrawal flow
- Batch settlement processing
- Validator rotation
- Emergency scenarios

### 3. Fuzzing
- Message validation
- Arithmetic operations
- State transitions
- Access control

### 4. Formal Verification
- Critical invariants
- Token conservation
- No locked funds

## Economic Model

### Fee Structure
- **Deposit fees**: 0.1% (for system maintenance)
- **Withdrawal fees**: Flat fee + percentage
- **Settlement fees**: Paid by traders in SyndDB
- **TEE Relayer funding**: Covered by protocol treasury (no profit motive needed)

### Incentive Alignment
- **Validators**: Fees from settlements
- **TEE Relayer**: Operational costs covered by protocol
- **Users**: Fast finality and low fees
- **Sequencer**: Volume-based revenue

## Risk Parameters

### Initial Configuration
- **Min validators**: 5
- **Signature threshold (m)**: 3
- **Daily withdrawal limit**: $10M
- **Per-user daily limit**: $1M
- **Large withdrawal delay**: 4 hours
- **Batch size limit**: 100 updates

### Adjustment Mechanism
- Governance can update parameters
- Time delay for changes
- Emergency multisig override
- Automatic circuit breakers

## Success Metrics

### 1. Security
- Zero funds lost
- No successful attacks
- 100% withdrawal success rate

### 2. Performance
- \<5 minute withdrawal time
- \<$10 withdrawal cost
> - 1000 settlements/day capacity

### 3. Adoption
> - $100M TVL within 6 months
> - 10 active validators
> - 1-3 TEE relayers (redundancy for availability)

## Dependencies

- OpenZeppelin contracts v5.0+
- SP1 verifier contracts
- Lit Protocol contracts
- Chainlink oracles (optional)
- Gnosis Safe (for admin)

## Audit Requirements

### 1. Code Audit
- Two independent firms
- Focus on access control
- Economic attack vectors
- Gas optimization

### 2. Economic Audit
- Incentive analysis
- Game theory review
- Stress testing

### 3. Formal Verification
- Key invariants
- State machine correctness
- No deadlocks

## Conclusion

This Bridge architecture provides a secure, efficient, and flexible solution for SyndDB's unique requirements. The combination of TEE validators, permissioned TEE relayers, and multi-signature verification creates a highly secure system that eliminates common attack vectors like transaction withholding and front-running. The dual-layer TEE approach (validators + relayer) ensures both state attestation and transaction submission are protected.

The WASM version pinning mechanism in Chain.sol enables seamless upgrades without replacing TEE infrastructure - TEEs simply bootstrap from the pinned version stored on IPFS/Arweave. This allows for rapid iteration and bug fixes while maintaining the security guarantees of the TEE environment.

The support for diverse use cases - from DeFi orderbooks to gaming assets to DePIN telemetry - distinguishes this from simple token bridges. The separation of concerns between Bridge.sol (token movement), Chain.sol (state management and version control), and the WASM execution layer follows proven patterns from the OP Stack, enabling modular development and maintenance while supporting SyndDB's high-performance database capabilities across multiple domains.