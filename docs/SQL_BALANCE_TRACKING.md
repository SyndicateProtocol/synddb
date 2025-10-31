# SQL-Native Balance Tracking for SyndDB Bridge

## Overview

This document describes the SQL-native balance tracking system for the SyndDB bridge. Instead of using Merkle trees (which don't map well to SQL databases), we use a hybrid approach combining hash chains and balance state commitments that leverage SQLite's natural strengths.

## Core Principles

1. **Monotonic Sequence IDs**: SQLite's `AUTOINCREMENT` provides natural ordering
2. **Hash Chains**: Each balance update creates a hash chain for temporal ordering and tamper-evidence
3. **Balance State Snapshots**: Periodic snapshots of all balances for validator attestation
4. **Validator Consensus**: Validators attest to the entire balance state, not individual Merkle proofs
5. **Multi-Token Support**: Each account can have balances in multiple tokens simultaneously, tracked with composite key `(account_id, token_address)`

## Multi-Token Architecture

### How Multi-Token Support Works

The system tracks balances for **multiple tokens per account** using a composite key approach:

1. **Storage**: Each `(account_id, token_address)` pair gets its own row
2. **Global Commitment**: A single `balanceStateHash` commits to **all** account-token pairs
3. **Token-Specific Withdrawals**: Each withdrawal references a specific token and its balance
4. **Independent Balances**: An account's ETH balance is separate from its USDC balance, etc.

### Example Multi-Token State

```
Alice's balances:
- (0xAlice, 0x0, 5 ETH)           ← ETH balance
- (0xAlice, 0xUSDC, 100 USDC)      ← USDC balance
- (0xAlice, 0xDAI, 50 DAI)         ← DAI balance

Bob's balances:
- (0xBob, 0xUSDC, 200 USDC)        ← USDC balance
- (0xBob, 0xWBTC, 0.5 WBTC)        ← WBTC balance
```

All 5 entries are hashed together into a single `balanceStateHash`:
```
balanceStateHash = keccak256(
    (0xAlice, 0x0, 5 ETH) ||
    (0xAlice, 0xDAI, 50 DAI) ||
    (0xAlice, 0xUSDC, 100 USDC) ||
    (0xBob, 0xUSDC, 200 USDC) ||
    (0xBob, 0xWBTC, 0.5 WBTC)
)
```

Validators attest to this **single hash** that covers all tokens, but withdrawals are processed **per-token** with the specific balance for that token.

## SQLite Schema

### Balance Updates Table (Audit Trail)

```sql
CREATE TABLE balance_updates (
    sequence_id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL,
    token_address TEXT NOT NULL,
    amount INTEGER NOT NULL,
    update_type TEXT CHECK(update_type IN ('credit', 'debit')) NOT NULL,
    balance_before INTEGER NOT NULL,
    balance_after INTEGER NOT NULL,
    state_hash TEXT NOT NULL,  -- H(prev_state_hash || this_update)
    created_at INTEGER NOT NULL,
    tx_hash TEXT,  -- Optional: L1 tx that triggered this
    INDEX idx_account_token (account_id, token_address),
    INDEX idx_created_at (created_at)
);
```

**Hash Chain Computation:**
```python
def compute_state_hash(prev_state_hash, account_id, token, amount, update_type, balance_after):
    data = prev_state_hash + account_id + token + str(amount) + update_type + str(balance_after)
    return keccak256(data)
```

### Account Balances Table (Current State)

```sql
CREATE TABLE account_balances (
    account_id TEXT NOT NULL,
    token_address TEXT NOT NULL,  -- ETH = 0x0, ERC20s = token contract address
    balance INTEGER NOT NULL CHECK(balance >= 0),
    last_sequence_id INTEGER NOT NULL,
    last_updated INTEGER NOT NULL,
    PRIMARY KEY (account_id, token_address),  -- COMPOSITE KEY: One row per (account, token) pair
    FOREIGN KEY (last_sequence_id) REFERENCES balance_updates(sequence_id)
);
```

**Multi-Token Examples:**
```sql
-- Alice's balances across multiple tokens
INSERT INTO account_balances VALUES
    ('0xAlice', '0x0000000000000000000000000000000000000000', 5000000000000000000, 1001, 1730000000),  -- 5 ETH
    ('0xAlice', '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48', 100000000, 1002, 1730000001),             -- 100 USDC (6 decimals)
    ('0xAlice', '0x6B175474E89094C44Da98b954EedeAC495271d0F', 50000000000000000000, 1003, 1730000002);  -- 50 DAI (18 decimals)

-- Bob's balances
INSERT INTO account_balances VALUES
    ('0xBob', '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48', 200000000, 1004, 1730000003),    -- 200 USDC
    ('0xBob', '0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599', 50000000, 1005, 1730000004);     -- 0.5 WBTC (8 decimals)

-- Query all tokens for a user
SELECT token_address, balance FROM account_balances WHERE account_id = '0xAlice';

-- Query specific token balance for a user
SELECT balance FROM account_balances
WHERE account_id = '0xAlice' AND token_address = '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48';
```

### State Commitments Table (On-Chain Sync)

```sql
CREATE TABLE state_commitments (
    state_version INTEGER PRIMARY KEY AUTOINCREMENT,
    sequence_id_range_start INTEGER NOT NULL,
    sequence_id_range_end INTEGER NOT NULL,
    hash_chain_head TEXT NOT NULL,  -- Last state_hash from balance_updates
    balance_state_hash TEXT NOT NULL,  -- keccak256(all balances sorted)
    total_accounts INTEGER NOT NULL,
    committed_at INTEGER NOT NULL,
    block_number INTEGER,  -- L1 block where this was committed
    tx_hash TEXT,  -- L1 tx hash of commitment
    FOREIGN KEY (sequence_id_range_end) REFERENCES balance_updates(sequence_id)
);
```

## Balance State Hash Computation

The balance state hash is a deterministic hash of **all account-token balances across all tokens**:

```python
def compute_balance_state_hash(db_cursor):
    # Get ALL balances for ALL tokens sorted deterministically
    # Each row represents one (account, token) pair
    rows = db_cursor.execute("""
        SELECT account_id, token_address, balance
        FROM account_balances
        WHERE balance > 0
        ORDER BY account_id ASC, token_address ASC  -- Critical: deterministic ordering
    """).fetchall()

    # Pack into bytes: Each entry is 72 bytes (20 + 20 + 32)
    packed_data = b""
    for account_id, token_address, balance in rows:
        # Convert to bytes (account=20 bytes, token=20 bytes, balance=32 bytes)
        account_bytes = bytes.fromhex(account_id[2:])  # Remove 0x prefix
        token_bytes = bytes.fromhex(token_address[2:])
        balance_bytes = balance.to_bytes(32, byteorder='big')
        packed_data += account_bytes + token_bytes + balance_bytes

    return keccak256(packed_data)
```

**Multi-Token Example:**
```python
# Database state:
# (0xAlice, 0x0 [ETH], 5000000000000000000)
# (0xAlice, 0xDAI, 50000000000000000000)
# (0xAlice, 0xUSDC, 100000000)
# (0xBob, 0xUSDC, 200000000)
# (0xBob, 0xWBTC, 50000000)

# Packed data (360 bytes total = 5 entries × 72 bytes):
packed = (
    bytes.fromhex('Alice...')[:20] + bytes.fromhex('0x0...')[:20] + (5_000_000_000_000_000_000).to_bytes(32, 'big') +
    bytes.fromhex('Alice...')[:20] + bytes.fromhex('DAI...')[:20] + (50_000_000_000_000_000_000).to_bytes(32, 'big') +
    bytes.fromhex('Alice...')[:20] + bytes.fromhex('USDC...')[:20] + (100_000_000).to_bytes(32, 'big') +
    bytes.fromhex('Bob...')[:20] + bytes.fromhex('USDC...')[:20] + (200_000_000).to_bytes(32, 'big') +
    bytes.fromhex('Bob...')[:20] + bytes.fromhex('WBTC...')[:20] + (50_000_000).to_bytes(32, 'big')
)

balance_state_hash = keccak256(packed)  # Single hash commits to ALL balances
```

**Properties:**
- **Deterministic**: Same balances always produce same hash
- **Multi-Token**: Covers all tokens in a single hash
- **Order-independent of insertion**: Always sorted by (account, token)
- **Token-specific**: Each (account, token) pair is separate
- **Efficient to compute**: Single sequential scan
- **Validator consensus**: All validators independently compute and must agree

## Smart Contract Integration

### On-Chain Structs

```solidity
struct BalanceStateCommitment {
    uint256 stateVersion;           // Monotonic version from SQLite
    uint256 sequenceIdRangeStart;   // First sequence_id in this commitment
    uint256 sequenceIdRangeEnd;     // Last sequence_id in this commitment
    bytes32 hashChainHead;          // Last state_hash from balance_updates
    bytes32 balanceStateHash;       // keccak256 of all (account, token, balance)
    uint256 totalAccounts;          // Number of unique accounts with balances
    uint256 timestamp;              // When this commitment was created
}
```

### Commitment Process

1. **SQLite generates commitment**:
   ```sql
   INSERT INTO state_commitments (
       sequence_id_range_start,
       sequence_id_range_end,
       hash_chain_head,
       balance_state_hash,
       total_accounts,
       committed_at
   ) VALUES (?, ?, ?, ?, ?, ?);
   ```

2. **Sequencer signs commitment**:
   ```javascript
   const commitment = {
       stateVersion,
       sequenceIdRangeStart,
       sequenceIdRangeEnd,
       hashChainHead,
       balanceStateHash,
       totalAccounts,
       timestamp
   };
   const signature = await sequencer.signTypedData(domain, types, commitment);
   ```

3. **Validators verify and co-sign**:
   - Each validator independently computes balance_state_hash from their replica
   - If matches, validator signs the commitment
   - TEE relayer collects m-of-n signatures

4. **Relayer submits to L1**:
   ```solidity
   bridge.commitBalanceState(
       commitment,
       sequencerSignature,
       validatorSignatures
   );
   ```

### Withdrawal Process (Multi-Token Support)

**Important**: Each withdrawal specifies a **specific token**. The `accountBalance` parameter is the balance for **that specific token only**.

```solidity
function processWithdrawal(
    uint256 nonce,
    address recipient,
    address token,           // ← Specific token being withdrawn (e.g., USDC address)
    uint256 amount,          // ← Amount to withdraw in that token
    uint256 stateVersion,    // ← Which committed state this is based on
    uint256 accountBalance,  // ← Account's balance of THIS SPECIFIC TOKEN at that state
    uint256 deadline,
    bytes memory sequencerSignature,
    bytes[] memory validatorSignatures
) external onlyRelayer {
    // Verify state version is committed
    require(stateVersion <= latestBalanceStateVersion, "Future state");
    require(balanceStateCommitments[stateVersion].stateVersion == stateVersion, "Not committed");

    // Verify amount doesn't exceed attested balance FOR THIS TOKEN
    require(amount <= accountBalance, "Exceeds balance");

    // Sequencer and validators attest to:
    // (nonce, recipient, token, amount, stateVersion, accountBalance, deadline)
    //
    // Example: Alice withdraws 50 USDC at state v1000
    // - token = 0xUSDC
    // - amount = 50_000_000 (50 USDC with 6 decimals)
    // - accountBalance = 100_000_000 (Alice had 100 USDC at v1000)
    // - This doesn't affect her ETH, DAI, or other token balances

    // ... signature verification ...
    // ... execute withdrawal ...
}
```

**Multi-Token Withdrawal Example:**

At state version 1000, Alice has:
- 5 ETH (balance = 5000000000000000000)
- 100 USDC (balance = 100000000)
- 50 DAI (balance = 50000000000000000000)

All three balances are included in `balanceStateHash` at v1000.

Alice can make three separate withdrawals:

```javascript
// Withdraw 2 ETH
processWithdrawal(
    nonce: 1,
    recipient: 0xAlice,
    token: 0x0,                         // ETH
    amount: 2000000000000000000,         // 2 ETH
    stateVersion: 1000,
    accountBalance: 5000000000000000000, // Alice had 5 ETH at v1000
    ...
);

// Withdraw 50 USDC
processWithdrawal(
    nonce: 2,
    recipient: 0xAlice,
    token: 0xUSDC,                      // USDC token address
    amount: 50000000,                    // 50 USDC
    stateVersion: 1000,
    accountBalance: 100000000,           // Alice had 100 USDC at v1000
    ...
);

// Withdraw 25 DAI
processWithdrawal(
    nonce: 3,
    recipient: 0xAlice,
    token: 0xDAI,                       // DAI token address
    amount: 25000000000000000000,        // 25 DAI
    stateVersion: 1000,
    accountBalance: 50000000000000000000, // Alice had 50 DAI at v1000
    ...
);
```

Each withdrawal is independent and token-specific, but all reference the same committed `balanceStateHash` that includes all tokens.

## Example Workflow

### 1. User Deposits 100 USDC

```sql
-- Update balance
UPDATE account_balances
SET balance = balance + 100000000, -- 100 USDC (6 decimals)
    last_sequence_id = last_sequence_id + 1,
    last_updated = unixepoch()
WHERE account_id = '0xAlice' AND token_address = '0xUSDC';

-- Record update in audit trail
INSERT INTO balance_updates (
    account_id,
    token_address,
    amount,
    update_type,
    balance_before,
    balance_after,
    state_hash,
    created_at
) VALUES (
    '0xAlice',
    '0xUSDC',
    100000000,
    'credit',
    0,
    100000000,
    keccak256(prev_hash || '0xAlice' || '0xUSDC' || '100000000' || 'credit' || '100000000'),
    unixepoch()
);
```

### 2. Periodic State Commitment (Every 1000 Updates)

```python
# Query current state
cursor.execute("SELECT MAX(sequence_id) FROM balance_updates")
sequence_id_end = cursor.fetchone()[0]

# Get last commitment
cursor.execute("SELECT sequence_id_range_end FROM state_commitments ORDER BY state_version DESC LIMIT 1")
sequence_id_start = cursor.fetchone()[0] + 1 if cursor.fetchone() else 1

# Get hash chain head
cursor.execute("SELECT state_hash FROM balance_updates WHERE sequence_id = ?", (sequence_id_end,))
hash_chain_head = cursor.fetchone()[0]

# Compute balance state hash
balance_state_hash = compute_balance_state_hash(cursor)

# Count accounts
cursor.execute("SELECT COUNT(*) FROM account_balances WHERE balance > 0")
total_accounts = cursor.fetchone()[0]

# Create commitment
commitment = {
    "stateVersion": next_version,
    "sequenceIdRangeStart": sequence_id_start,
    "sequenceIdRangeEnd": sequence_id_end,
    "hashChainHead": hash_chain_head,
    "balanceStateHash": balance_state_hash,
    "totalAccounts": total_accounts,
    "timestamp": int(time.time())
}

# Validators sign
# Submit to L1
```

### 3. User Requests Withdrawal

```python
# Get latest committed state
commitment = get_latest_committed_state()

# Get user's balance at that state
cursor.execute("""
    SELECT balance
    FROM account_balances
    WHERE account_id = ? AND token_address = ?
""", (user_account, token))
account_balance = cursor.fetchone()[0]

# Create withdrawal request
withdrawal = {
    "nonce": next_nonce,
    "recipient": user_address,
    "token": token,
    "amount": withdrawal_amount,
    "stateVersion": commitment.stateVersion,
    "accountBalance": account_balance,
    "deadline": timestamp + 1_hour
}

# Sequencer signs, validators co-sign, relayer submits
```

## Security Properties

1. **Tamper Evidence**: Hash chain ensures any modification to history is detectable
2. **Validator Consensus**: m-of-n validators must agree on balance state
3. **Monotonic Versions**: State versions only increase, preventing rollbacks
4. **Balance Proofs**: Validators attest to specific account balances at specific versions
5. **Circuit Breakers**: On-chain withdrawal limits provide additional safety
6. **TEE Protection**: Sequencer and validators run in Trusted Execution Environments

## Performance Characteristics

- **Commitment Computation**: O(n) where n = number of accounts with balances
- **Hash Chain Updates**: O(1) per balance update
- **On-Chain Storage**: O(1) per state version (not per account)
- **Withdrawal Verification**: O(1) on-chain (just signature checks + lookup)

## Advantages Over Merkle Trees

1. **SQL-Native**: Uses SQLite's natural strengths (AUTOINCREMENT, ORDER BY)
2. **Simple**: No complex tree maintenance or rebalancing
3. **Efficient**: Single sequential scan to compute state hash
4. **Auditable**: Full history in balance_updates table
5. **Deterministic**: Independent validators compute same hashes
6. **Flexible**: Easy to add new fields or indexes

## Comparison Table

| Feature | Merkle Tree | Hash Chain + State Commitment |
|---------|-------------|------------------------------|
| Individual Proofs | ✅ Yes (log n) | ❌ No (not needed) |
| Multi-Token Support | ✅ Separate tree per token | ✅ Single hash for all tokens |
| SQL-Native | ❌ Complex | ✅ Yes |
| Update Complexity | O(log n) per token | O(1) per token |
| Storage Overhead | High (tree nodes) | Low (linear) |
| Validator Independence | ✅ Verify proofs | ✅ Compute own hash |
| Audit Trail | ❌ Limited | ✅ Complete |
| Token Addition | Requires new tree | Automatic (new rows) |
| Implementation | Complex | Simple |

## Future Enhancements

1. **Parallel Commitment**: Split accounts into shards for parallel hash computation
2. **Incremental Hashing**: Only rehash changed accounts since last commitment
3. **Compression**: Use Patricia trie for balance state hash
4. **ZK Proofs**: Add optional ZK proofs for privacy-preserving withdrawals
5. **State Diffs**: Instead of full balance hash, commit to balance diffs

## References

- EIP-712: Typed structured data hashing and signing
- SQLite AUTOINCREMENT: https://www.sqlite.org/autoinc.html
- Hash Chains: https://en.wikipedia.org/wiki/Hash_chain
- Optimistic Rollups: State commitments without fraud proofs
