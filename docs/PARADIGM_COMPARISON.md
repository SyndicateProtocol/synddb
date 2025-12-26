# SQLite vs Message-Passing Paradigm Comparison

This document compares the developer experience and trade-offs between two approaches to building stateful applications:

1. **SQLite Paradigm** (`origin/example-app` branch): Direct database operations with optional SyndDB replication
2. **Message-Passing Paradigm** (this branch): Submit messages to validators for on-chain execution

Both branches implement the same two applications:
- **Price Oracle**: Fetches prices from external sources, stores/publishes them
- **Prediction Market**: Binary YES/NO markets with simplified 50/50 pricing

---

## Executive Summary

| Aspect | SQLite | Message-Passing |
|--------|--------|-----------------|
| **Latency** | ~1ms | Seconds to minutes |
| **Consistency** | Strong (ACID) | Eventual |
| **Error Handling** | Simple try/catch | Multi-layer with retry logic |
| **Offline Capability** | Full | Read-only (local cache) |
| **State Location** | Local database | On-chain (with local cache) |
| **Network Dependencies** | Optional (replication) | Required (validators) |
| **Code Complexity** | Lower | Higher (~2-3x more code) |

---

## Code Comparison

### Price Oracle

| Metric | SQLite (`example-app`) | Message-Passing (`this branch`) |
|--------|------------------------|--------------------------------|
| Core oracle logic | ~300 lines (Python) | ~265 lines (Python) |
| Bridge/replication code | ~363 lines | ~550 lines |
| Total application | ~2,850 lines | ~1,370 lines* |

*Message-passing version excludes HTTP server and some features present in SQLite version.

### Prediction Market

| Metric | SQLite (`example-app`) | Message-Passing (`this branch`) |
|--------|------------------------|--------------------------------|
| Trading logic | ~323 lines (Rust) | Handled by Solidity contract |
| Application facade | ~408 lines | N/A (split across modules) |
| Bridge client | N/A | ~430 lines |
| Event sync | N/A | ~470 lines |
| Total Rust code | ~3,050 lines | ~2,640 lines |
| Solidity contract | N/A | ~370 lines |

---

## Write Operations

### SQLite Paradigm

```rust
// Synchronous, atomic, instant feedback
let trade = app.buy_shares(account_id, market_id, "yes", 100)?;
println!("Trade executed: {:?}", trade);
// State is immediately consistent
```

```python
# Python equivalent
oracle.fetch_and_store()  # Fetches prices and stores atomically
oracle.publish()  # Optional: replicate to sequencer
```

**Characteristics:**
- Single function call
- Immediate return with result
- Transaction either commits or rolls back
- No network required for local operations
- State is consistent after call returns

### Message-Passing Paradigm

```rust
// Step 1: Submit to validator
let result = client.buy_shares(&market_id, &user, Outcome::Yes, 100).await?;

// Step 2: Check submission result
if !result.success {
    if result.is_retryable() {
        // Transient failure - may have been retried already
        return Err(anyhow!("Transient failure: {}", result.error_code.unwrap()));
    } else {
        // Permanent failure (validation error)
        return Err(anyhow!("Rejected: {}", result.error_message.unwrap()));
    }
}

// Step 3: Poll for on-chain execution
let status = client
    .wait_for_completion(&result.message_id.unwrap(), Duration::from_secs(60))
    .await?;

// Step 4: Check execution result
if !status.is_success() {
    return Err(anyhow!("Execution failed at stage {}", status.stage));
}

println!("Trade confirmed in block {}", status.block_number.unwrap());
```

**Characteristics:**
- Multi-step async process
- Must handle submission failures separately from execution failures
- Requires polling for completion status
- Network required for all writes
- State becomes consistent eventually

---

## Error Handling

### SQLite Paradigm

```rust
// Simple error handling
match app.buy_shares(account_id, market_id, "yes", 100) {
    Ok(trade) => println!("Success: {:?}", trade),
    Err(e) => println!("Failed: {}", e),
}
```

Errors are simple and final:
- Constraint violations (insufficient balance, market resolved)
- Connection errors (rare with local SQLite)
- Schema errors (programming bugs)

### Message-Passing Paradigm

```python
# Complex error classification
RETRYABLE_ERRORS = [
    "NETWORK_ERROR",
    "TIMEOUT",
    "VALIDATOR_UNAVAILABLE",
    "RATE_LIMITED",
    "NONCE_CONFLICT",
]

async def push_price_with_retry(self, ...):
    attempts = 0
    delay = self.retry_delay

    while attempts < self.max_retries:
        attempts += 1
        result = await self._submit_message_detailed(message)

        if result.success:
            return result

        if not result.is_retryable:
            # Permanent failure - stop immediately
            return result

        # Exponential backoff for transient errors
        await asyncio.sleep(delay)
        delay *= 2

    return PushResult(error_code="MAX_RETRIES_EXCEEDED")
```

Error categories:
1. **Retryable** (transient): Network timeouts, service unavailable, rate limits
2. **Non-retryable** (permanent): Validation failures, invalid parameters
3. **Execution failures**: On-chain revert, gas issues
4. **Timeout**: Polling exceeded time limit

---

## Message Lifecycle (Message-Passing Only)

Messages go through 9 stages:

| Stage | Name | Description |
|-------|------|-------------|
| 0 | Pending | Message received by validator |
| 1 | Validated | Passed validation rules |
| 2 | Signed | Has required signatures |
| 3 | Published | Sent to storage layer |
| 4 | Confirmed | Confirmed in storage |
| 5 | Submitted | Sent to chain |
| 6 | Executed | Confirmed on chain |
| 7 | Rejected | Failed validation |
| 8 | Failed | Execution failed |

SQLite transactions have only 2 states: committed or not.

---

## Read Operations

### SQLite Paradigm

```rust
// Direct database queries - fast, consistent
let markets = app.list_markets()?;
let positions = app.get_positions(account_id)?;
let balance = conn.query_row("SELECT balance FROM accounts WHERE id = ?", [id])?;
```

- Sub-millisecond latency
- Always consistent with latest writes
- Full SQL expressiveness

### Message-Passing Paradigm

Reads require a local cache synced from chain events:

```rust
// Read from local cache (not on-chain)
let markets = store.list_markets()?;

// Cache may be stale if sync is behind
// Must handle eventual consistency
```

Additional complexity:
- Event sync with idempotency handling
- Chain reorganization handling
- Gap detection for missed events

```rust
pub fn process_events(&mut self, conn: &Connection, events: Vec<ContractEvent>) -> Result<usize> {
    for event in events {
        // Check if already processed (idempotency)
        let exists = tx.query_row(
            "SELECT 1 FROM trades WHERE tx_hash = ?1 AND log_index = ?2",
            params![tx_hash, log_index],
            |_| Ok(true),
        ).unwrap_or(false);

        if exists { continue; }

        // Process event...
    }
}
```

---

## When to Use Each Paradigm

### Use SQLite When:

- **Performance is critical**: Sub-millisecond reads and writes
- **Strong consistency required**: Financial calculations, inventory management
- **Single-node acceptable**: No multi-region or decentralization requirements
- **Offline operation needed**: Mobile apps, edge computing
- **Rapid development**: Simpler debugging, testing, deployment
- **Complex queries**: Analytics, reporting, aggregations

### Use Message-Passing When:

- **On-chain execution required**: Smart contract state, blockchain guarantees
- **Multi-validator consensus**: Decentralized trust model
- **Cross-chain operations**: Bridge messages between networks
- **Audit trail on blockchain**: Immutable, verifiable history
- **External oracle integration**: Price feeds, external data sources
- **Regulatory requirements**: Verifiable execution by third parties

---

## Hybrid Approach (Demonstrated in This Branch)

Both prediction-market-bridge and price-oracle use a hybrid pattern:

1. **Writes**: Go through message-passing to validators for on-chain execution
2. **Reads**: Query local SQLite cache synced from chain events

This provides:
- On-chain guarantees for state mutations
- Fast reads from local database
- Complex query capabilities (joins, aggregations)

Trade-offs:
- Eventual consistency between chain and cache
- Additional complexity for event sync
- Must handle chain reorganizations

---

## Code Complexity Comparison

### SQLite Buy Shares

```rust
pub fn buy_shares(conn: &Connection, account_id: i64, market_id: i64,
                  outcome: &str, shares: i64) -> Result<Trade> {
    let tx = conn.unchecked_transaction()?;

    // Check balance
    let balance: i64 = conn.query_row(
        "SELECT balance FROM accounts WHERE id = ?1", [account_id], |r| r.get(0))?;

    if balance < total_cost {
        bail!("insufficient balance");
    }

    // Deduct balance
    tx.execute("UPDATE accounts SET balance = balance - ?1 WHERE id = ?2",
               [total_cost, account_id])?;

    // Update position
    tx.execute("INSERT INTO positions ... ON CONFLICT DO UPDATE ...", [...])?;

    // Record trade
    tx.execute("INSERT INTO trades ...", [...])?;

    tx.commit()?;
    Ok(trade)
}
```

~80 lines total, synchronous, atomic.

### Message-Passing Buy Shares

Involves:
1. **ABI Encoding** (~50 lines): Encode function call for EVM
2. **Bridge Client** (~100 lines): HTTP submission with retry
3. **Status Polling** (~50 lines): Wait for execution
4. **Event Sync** (~100 lines): Update local cache from chain events
5. **Solidity Contract** (~50 lines): On-chain execution

~350 lines total, async, multi-stage.

---

## Recommendations

1. **Start with SQLite** for rapid prototyping and development
2. **Add message-passing** when on-chain guarantees become necessary
3. **Use the hybrid pattern** for production systems needing both
4. **Invest in event sync infrastructure** early if using message-passing
5. **Build comprehensive retry and timeout handling** from the start

The additional complexity of message-passing is justified when decentralized execution and blockchain guarantees are core requirements. For most applications, SQLite provides better developer experience with lower operational overhead.
