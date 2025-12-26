# Prediction Market: Message-Passing Bridge Paradigm

This example demonstrates a binary prediction market built using the message-passing paradigm, where all state mutations flow through a Bridge validator before being executed on-chain.

## Purpose

Compare the developer experience and ergonomics of message-passing vs direct SQLite operations for stateful applications. This serves as a counterpoint to the SQLite-based prediction market on the `example-app` branch.

## Architecture Overview

```
┌─────────────┐     ┌──────────────┐     ┌────────────────┐     ┌───────────┐
│   Client    │────▶│   Validator  │────▶│  On-Chain TX   │────▶│  Contract │
│  (CLI/HTTP) │     │   (HTTP)     │     │  (Execution)   │     │  (State)  │
└─────────────┘     └──────────────┘     └────────────────┘     └───────────┘
       │                                                              │
       │                  ┌──────────────┐                            │
       └─────────────────▶│ Local Cache  │◀───────────────────────────┘
         (reads)          │  (SQLite)    │      (event sync)
                          └──────────────┘
```

### Data Flow

1. **Writes**: Client → Validator HTTP API → Signature collection → On-chain execution
2. **Reads**: Client → Local SQLite cache (synced from chain events)

This hybrid approach is realistic for production: you cannot avoid local state for fast reads, but writes must go through the Bridge for validation and execution.

## Ergonomic Comparison

### SQLite Version (Direct Operations)

```rust
// Synchronous, instant, atomic
let trade = app.buy_shares(account_id, market_id, "yes", 100)?;
println!("Trade executed: {:?}", trade);
// Done - state is immediately consistent
```

**Characteristics**:
- Instant feedback
- Atomic transactions
- Simple error handling
- No network concerns
- Strong consistency

### Message-Passing Version (This Example)

```rust
// Step 1: Submit to validator
let result = client.buy_shares(&market_id, &user, Outcome::Yes, 100).await?;

// Step 2: Check submission result
if !result.success {
    if result.is_retryable() {
        return Err(anyhow!("Transient failure: {}", result.error_code.unwrap()));
    } else {
        return Err(anyhow!("Rejected: {}", result.error_message.unwrap()));
    }
}

// Step 3: Poll for on-chain execution
let status = client
    .wait_for_completion(&result.message_id.unwrap(), Duration::from_secs(60), Duration::from_secs(2))
    .await?;

// Step 4: Check execution result
if !status.is_success() {
    return Err(anyhow!("Execution failed: stage {}", status.stage));
}

println!("Trade confirmed in block {}", status.block_number.unwrap());

// Step 5: Wait for local cache to sync (or force sync)
// Local cache may still show stale state briefly
```

**Characteristics**:
- Asynchronous by nature
- Multi-step process
- Network failures possible at each step
- Requires retry logic with exponential backoff
- Error classification (retryable vs permanent)
- Eventual consistency
- Polling overhead

## Key Challenges Demonstrated

### 1. Async Operations

Every write operation requires:
- HTTP request to validator
- Wait for signature collection
- Wait for on-chain execution
- Poll for completion status

### 2. Error Classification

Must distinguish between:
- **Retryable errors**: Network timeouts, validator unavailable, rate limits
- **Permanent errors**: Insufficient balance, market resolved, invalid parameters

```rust
const RETRYABLE_ERRORS: &[&str] = &[
    "NETWORK_ERROR",
    "TIMEOUT",
    "VALIDATOR_UNAVAILABLE",
    "RATE_LIMITED",
    "NONCE_CONFLICT",
];
```

### 3. Retry Logic

```rust
pub async fn submit_with_retry(&self, calldata: Vec<u8>) -> Result<PushResult> {
    let mut last_error = None;

    for attempt in 0..self.max_retries {
        match self.submit_message(calldata.clone()).await {
            Ok(result) => {
                if result.success || !result.is_retryable() {
                    return Ok(result);
                }
                last_error = Some(result);
            }
            Err(e) => {
                last_error = Some(PushResult::network_error(e.to_string()));
            }
        }

        let delay = self.retry_delay * 2u32.pow(attempt);
        tokio::time::sleep(delay).await;
    }

    Ok(last_error.unwrap())
}
```

### 4. Event Sync Complexity

Local cache must be kept in sync with on-chain state:

```rust
pub fn process_events(&mut self, conn: &Connection, events: Vec<ContractEvent>) -> Result<usize> {
    // Must be idempotent (same event can be processed multiple times)
    // Must handle events in order
    // Must update multiple tables atomically
    // Must track sync progress

    for event in events {
        match event {
            ContractEvent::SharesPurchased { tx_hash, log_index, .. } => {
                // Check if already processed (idempotency)
                let exists = tx.query_row(
                    "SELECT 1 FROM trades WHERE tx_hash = ?1 AND log_index = ?2",
                    params![tx_hash, log_index],
                    |_| Ok(true),
                ).unwrap_or(false);

                if exists { continue; }

                // Update position, market totals, account balance, record trade...
            }
        }
    }
}
```

### 5. Eventual Consistency

After submitting a trade:
- On-chain state updates immediately after tx confirmation
- Local cache may lag by seconds/minutes
- UI must handle "pending" states gracefully

### 6. Chain Reorganizations

Must handle reorgs that invalidate previously "confirmed" transactions:

```rust
pub fn handle_reorg(&mut self, conn: &Connection, reorg_block: u64) -> Result<()> {
    // Delete trades from reorged blocks
    tx.execute("DELETE FROM trades WHERE block_number >= ?1", params![reorg_block])?;

    // Recalculate affected positions (complex)
    // Reset sync state
}
```

## Message Lifecycle

Messages go through 9 stages:

| Stage | Description |
|-------|-------------|
| 0 | Pending - Message received |
| 1 | Validated - Passed validation rules |
| 2 | Signed - Has required signatures |
| 3 | Published - Sent to storage layer |
| 4 | Confirmed - Confirmed in storage |
| 5 | Submitted - Sent to chain |
| 6 | Executed - Confirmed on chain |
| 7 | Rejected - Failed validation |
| 8 | Failed - Execution failed |

## Solidity Contract

The on-chain contract receives pre-validated messages:

```solidity
contract PredictionMarket {
    modifier onlyBridge() {
        require(msg.sender == bridge, "Unauthorized");
        _;
    }

    function buyShares(
        bytes32 marketId,
        address user,
        uint8 outcome,
        uint256 shares
    ) external onlyBridge {
        // All validation already done by Bridge
        // Just execute state changes
    }
}
```

## When to Use Each Paradigm

### Use SQLite (Direct Operations) When:
- Fast reads and writes are critical
- Strong consistency required
- Single-node deployment acceptable
- No blockchain integration needed
- Development/prototyping phase

### Use Message-Passing When:
- On-chain execution required
- Multi-validator consensus needed
- Cross-chain operations
- Audit trail on blockchain
- Decentralized trust model

## Running the Example

```bash
# Initialize local cache
cargo run -p prediction-market-bridge -- init

# Create a market (requires Bridge validator)
cargo run -p prediction-market-bridge -- \
  --domain 0x... \
  create-market --id market1 --question "Will BTC hit 100k?" --resolution-time 1800000000

# Buy shares
cargo run -p prediction-market-bridge -- \
  --domain 0x... \
  buy --market market1 --user 0xalice --outcome yes --shares 100

# Query local cache (no Bridge needed)
cargo run -p prediction-market-bridge -- markets
cargo run -p prediction-market-bridge -- portfolio 0xalice
```

## Conclusion

The message-passing paradigm adds significant complexity compared to direct SQLite operations:

| Aspect | SQLite | Message-Passing |
|--------|--------|-----------------|
| Latency | ~1ms | Seconds to minutes |
| Consistency | Strong | Eventual |
| Error handling | Simple | Multi-layer |
| Retry logic | None needed | Required |
| State sync | N/A | Complex |
| Network deps | None | High |
| Lines of code | ~500 | ~1500 |

This complexity is the cost of decentralized execution. The trade-off is appropriate when on-chain guarantees and multi-party validation are required.
