# PLAN\_BRIDGE.md - SyndDB Bridge Smart Contract Architecture

## Executive Summary
The Bridge.sol contract serves as the settlement layer interface for SyndDB, enabling secure asset bridging between the high-performance database layer and the blockchain. It supports complex state transitions for onchain orderbooks, requires multi-signature validation from TEE validators, and allows permissionless relaying of signed messages.

<!-- Not all bridge use cases will be onchain orderbooks, just some of them. It supports message passing to/from the SyndDB database, for use cases including onchain orderbooks for DeFi but also onchain telemetry for DePIN, onchain assets for gaming, onchain social data, etc. -->


## Core Architecture Principles

### 1. Asymmetric Trust Model
- **Deposits**: Trustless (any user can deposit)
- **Withdrawals**: Require m-of-n validator signatures + sequencer approval
- **State Updates**: Batched settlement for orderbook rebalancing

<!-- The sequencer approval phrasing makes it sound like the sequencer approves as a last step. Instead, make it clear that the validators are only able to sign off on a signed sequencer message, and nothing else, for withdrawals.

Also make it clear that this is not only orderbooks. It's just batched updates for message passing in general. -->

### 2. Permissionless Relaying
- Anyone can submit signed messages to the bridge
- Relayers are incentivized through fee rebates
- Messages include nonces to prevent replay attacks

<!-- Can we have batching live at the relayer level? That seems cleaner than having validators batch, since validator batching requires validators to be aware of past signature state/block times, while the relayer is better suited for that via separation of concerns. -->


### 3. Complex State Transitions
- Not limited to 1:1 token swaps
- Supports orderbook settlement with multiple balance updates
- Handles partial fills and complex trading outcomes

<!-- Not just orderbooks! Orderbooks are just an example -->


## Key Components

### 1. Deposit System
- **Direct deposits**: Users lock tokens in bridge
- **Deposit receipts**: Emitted for sequencer to credit in SyndDB
- **Multi-token support**: ETH, ERC-20, potentially ERC-721/1155
- **Deposit limits**: Per-user and global circuit breakers

<!-- Explain that this can be treated as a lock (unlocked upon settlement) or a bridge (tokens are expected to stay on the platform for a long time and eventually withdraw). Most use cases look more like locking than bridging, where it is short-lived until settlement rather than long-lived.

Make sure that deposit limits are configurable and are simply for risk reduction on new launches, while the WASM logic is still early. -->


### 2. Validator Registry
- **TEE attestation verification**: Validators prove TEE environment
- **Dual attestation**: SP1 proofs + Lit Protocol verification
- **Dynamic set**: Validators can be added/removed by governance
- **Key rotation**: Support for validator key updates

<!-- This can be permissioned right now. Validators should be able to permissionlessly propose being added if they submit the appropriate SP1 proof and Lit Protocol attestation that their TEE is running, but adding them should be permissioned.

Explain that the dual attestation is useful for TEE + zkVM combination. -->


### 3. Message Processing
- **Sequencer messages**: Signed state updates from sequencer
- **Validator confirmations**: m-of-n threshold signatures required 
<!-- m-of-n signatures required to validate sequencer messages -->

- **Message types**:
  - Withdrawals (single user, single token)
  - Batch settlements (multiple users, multiple tokens)
  - Rebalancing (orderbook state reconciliation)
  - Emergency actions (pause, circuit breaker triggers)

<!-- We should be clear here that the Bridge.sol is just for message passing. It doesn't actually care about state updates. It just cares about tokens in -> tokens out. Diffs/snapshots/etc can live outside of Bridge.sol in a different file (Chain.sol?)

Do some research on how OP Stack handles separation of concerns and then come back to me on your recommendations. -->


### 4. Settlement Engine
- **Batch processing**: Multiple operations in single transaction
- **Atomic execution**: All-or-nothing settlement
- **Gas optimization**: Efficient storage patterns and batch operations
- **Slippage protection**: Max deviation from expected state

<!-- Is the batch processing tunable? For minimum latency, batch times should be equal to block times for a given chain (or Flashblocks block times for chains like Base). Many groups care more about latency than gas costs. Also make it clear that this is the tradeoff. -->


### 5. Security Mechanisms
- **Circuit breakers**: Daily/hourly withdrawal limits
- **Time delays**: Optional delay for large withdrawals
- **Pause functionality**: Emergency stop mechanism
- **Rate limiting**: Per-user withdrawal frequency limits
- **Merkle proofs**: For large batch settlements

<!-- Validators can also simply refuse to sign risky withdrawals. So in some ways these onchain circuit breakers complement possible offchain circuit breakers. -->


## Data Structures

```solidity
struct Validator {
    address publicKey;
    bytes32 attestationHash;
    bool isActive;
    uint256 addedAt;
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
3. Validators verify state and add signatures
4. Relayer submits message with m-of-n signatures 
<!-- Validator signatures. The sequencer signature is handled during bridge parsing -->

5. Bridge validates signatures and nonce

<!-- Both the global nonce and the per-user nonce. The global and per user nonce should be a mapping of nonce data to TX data, to ensure that a re-org doesn't cause validators to overwrite old nonces. Add to the security mechanisms that re-org detection is available by having validators halt signing if global and per-user nonces change.

Or maybe we call these accumulators? Do some research and come back with a recommendation. -->

7. Bridge transfers tokens to recipient
8. Bridge emits `Withdrawal` event

<!-- Do we want to consider renaming these lock/unlock? I'm a bit torn over whether this feels more like locking and unlocking or depositing and withdrawing. I think depositing and withdrawing is more accurate because there's not a 1:1 relationship between tokens deposited and tokens withdrawn. -->


### Batch Settlement Flow (Orderbook)
1. Sequencer computes net position changes after trading period
2. Sequencer creates merkle tree of balance updates
3. Sequencer signs batch settlement message
4. Validators verify orderbook state and sign
5. Relayer submits batch with merkle root
6. Bridge processes updates atomically
7. Net token movements executed on-chain

<!-- No, this should be using snapshot and diff updates. We need a second contract that represents tracked state (or a part of the Bridge, that works too) that only allows for signatures once the snapshots/diffs/etc have been written by the validators. Only then can withdrawals be processed, because the necessary state for withdrawals has been made available. -->


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
- **Mitigation**: Chain ID in signatures
<!-- For deposit -->


#### 4. Front-running
- **Mitigation**: Commit-reveal for deposits
- **Mitigation**: Deadlines on messages
- **Mitigation**: First-come-first-served processing
<!-- This is irrelevant, front-running isn't a concern  since most data is sent to the sequencer and deposits/withdrawals are not ordering-dependent. -->


#### 5. Gas Griefing
- **Mitigation**: Gas rebates for relayers
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

### For Relayers
- Monitor for signed messages
- Estimate gas costs
- Submit profitable transactions
- Handle revert scenarios

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
- **Deposit fees**: 0.1% (for relayer incentives)
- **Withdrawal fees**: Flat fee + percentage
- **Settlement fees**: Paid by traders in SyndDB
- **Relayer rewards**: Gas + premium

### Incentive Alignment
- **Validators**: Fees from settlements
- **Relayers**: Gas rebates + tips
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
> - 5 independent relayers

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

This Bridge architecture provides a secure, efficient, and flexible solution for SyndDB's unique requirements. The combination of TEE validators, multi-signature verification, and circuit breakers ensures security while maintaining the performance benefits of the SyndDB architecture. The support for complex orderbook operations distinguishes this from simple token bridges, enabling sophisticated DeFi applications to leverage SyndDB's high-performance database capabilities.