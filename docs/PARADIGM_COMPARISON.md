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

## Detailed Code Comparisons

### Example 1: Buying Shares in a Prediction Market

#### SQLite Version (80 lines, synchronous)

**File**: `origin/example-app:examples/prediction-market/src/trading.rs:8-85`

```rust
pub fn buy_shares(
    conn: &Connection,
    account_id: i64,
    market_id: i64,
    outcome: &str,
    shares: i64,
) -> Result<Trade> {
    // Validation
    if outcome != "yes" && outcome != "no" {
        bail!("outcome must be 'yes' or 'no'");
    }

    // Check market is not resolved
    let market_outcome: String = conn.query_row(
        "SELECT outcome FROM markets WHERE id = ?1",
        params![market_id],
        |row| row.get(0),
    )?;
    if market_outcome != "unresolved" {
        bail!("market {} is already resolved", market_id);
    }

    let total_cost = shares * PRICE_PER_SHARE;

    // Check balance
    let balance: i64 = conn.query_row(
        "SELECT balance FROM accounts WHERE id = ?1",
        params![account_id],
        |row| row.get(0),
    )?;
    if balance < total_cost {
        bail!("insufficient balance: need {} cents, have {} cents", total_cost, balance);
    }

    // Atomic transaction
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE accounts SET balance = balance - ?1 WHERE id = ?2",
               params![total_cost, account_id])?;
    tx.execute("INSERT INTO positions ... ON CONFLICT DO UPDATE ...", [...])?;
    tx.execute("INSERT INTO trades ...", [...])?;
    tx.commit()?;

    Ok(trade)  // Immediate result
}
```

**Caller code** (`origin/example-app:examples/prediction-market/src/app.rs:211-218`):
```rust
pub fn buy_shares(&self, account_id: i64, market_id: i64,
                  outcome: &str, shares: i64) -> Result<Trade> {
    trading::buy_shares(self.conn(), account_id, market_id, outcome, shares)
}
// That's it. One line.
```

#### Message-Passing Version (~350 lines across 4 files, async)

**Step 1: ABI Encoding** (`examples/prediction-market-bridge/src/bridge/encoding.rs:38-52`)
```rust
sol! {
    #[sol(rpc)]
    interface IPredictionMarket {
        function buyShares(bytes32 marketId, address user, uint8 outcome, uint256 shares) external;
    }
}

pub fn encode_buy_shares(market_id: [u8; 32], user: Address, outcome: u8, shares: u64) -> Vec<u8> {
    let call = IPredictionMarket::buySharesCall {
        marketId: market_id.into(),
        user,
        outcome,
        shares: U256::from(shares),
    };
    call.abi_encode()
}
```

**Step 2: Bridge Client with Retry** (`examples/prediction-market-bridge/src/bridge/client.rs:130-178`)
```rust
async fn submit_with_retry(&self, message_type: &str, calldata: Vec<u8>,
                           metadata: serde_json::Value) -> PushResult {
    let mut attempts = 0u32;
    let mut delay = self.retry_delay;

    while attempts < self.max_retries {
        attempts += 1;

        // Build fresh message with new nonce for each attempt
        let message = self.build_message(message_type, calldata.clone(), metadata.clone());
        let mut result = self.submit_message(message).await;
        result.attempts = attempts;

        if result.success {
            return result;
        }

        if !result.is_retryable {
            warn!(error_code = ?result.error_code,
                  "Non-retryable error after {} attempt(s)", attempts);
            return result;
        }

        if attempts < self.max_retries {
            info!(error_code = ?result.error_code, attempts, delay_ms = delay.as_millis(),
                  "Retryable error, waiting before retry");
            tokio::time::sleep(delay).await;
            delay *= 2; // Exponential backoff
        }
    }

    PushResult {
        success: false,
        error_code: Some("MAX_RETRIES_EXCEEDED".to_string()),
        error_message: Some(format!("Failed after {} attempts", attempts)),
        is_retryable: false,
        attempts,
    }
}
```

**Step 3: Status Polling** (`examples/prediction-market-bridge/src/bridge/client.rs:332-377`)
```rust
pub async fn wait_for_completion(
    &self,
    message_id: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<MessageStatus> {
    let start = std::time::Instant::now();
    let mut last_stage = None;

    while start.elapsed() < timeout {
        let status = self.get_message_status(message_id).await?;

        // Log stage transitions
        if last_stage != Some(status.stage) {
            debug!(message_id, stage = status.stage, status = %status.status,
                   "Message status update");
            last_stage = Some(status.stage);
        }

        if status.is_terminal() {
            return Ok(status);
        }

        tokio::time::sleep(poll_interval).await;
    }

    // Timeout - return last known status
    warn!(message_id, elapsed_ms = start.elapsed().as_millis(),
          "Timeout waiting for message completion");
    Ok(self.get_message_status(message_id).await?)
}
```

**Caller code** (`examples/prediction-market-bridge/src/main.rs:262-301`):
```rust
Commands::Buy { market, user, outcome, shares } => {
    let client = bridge_client.ok_or_else(||
        anyhow::anyhow!("--domain required for Bridge operations"))?;
    let outcome: Outcome = outcome.parse()?;
    let padded_id = pad_market_id(&market)?;

    info!("Submitting buyShares to Bridge...");
    let result = client.buy_shares(&padded_id, &user, outcome, shares).await?;

    if result.success {
        info!("Buy order submitted!");
        let msg_id = result.message_id.clone().unwrap_or_default();

        // Wait for completion
        if !msg_id.is_empty() {
            info!("Waiting for on-chain confirmation...");
            let status = client
                .wait_for_completion(&msg_id, Duration::from_secs(60), Duration::from_secs(2))
                .await?;

            if status.is_success() {
                info!("Trade confirmed on-chain!");
            } else {
                anyhow::bail!("Trade failed: {}", status.status);
            }
        }
    } else {
        anyhow::bail!("Buy rejected: {} - {}",
            result.error_code.unwrap_or_default(),
            result.error_message.unwrap_or_default());
    }
}
```

**Step 4: Event Sync for Local Cache** (`examples/prediction-market-bridge/src/sync.rs:180-257`)
```rust
ContractEvent::SharesPurchased {
    market_id, user, outcome, shares, block_number, tx_hash, log_index,
} => {
    let price = 50i64;
    let total = (*shares as i64) * price;

    // Check if already processed (idempotency via tx_hash + log_index)
    let exists: bool = tx
        .query_row(
            "SELECT 1 FROM trades WHERE tx_hash = ?1 AND log_index = ?2",
            params![tx_hash, *log_index as i64],
            |_| Ok(true),
        )
        .unwrap_or(false);

    if exists {
        debug!(tx_hash, log_index, "Trade already processed, skipping");
        continue;
    }

    // Update position
    tx.execute(
        "INSERT INTO positions (user, market_id, outcome, shares, cost_basis)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(user, market_id, outcome) DO UPDATE SET
           shares = shares + excluded.shares,
           cost_basis = cost_basis + excluded.cost_basis",
        params![user, market_id, outcome.as_u8() as i64, *shares as i64, total],
    )?;

    // Update market totals, deduct balance, record trade...
    // (Additional ~30 lines of SQL updates)
}
```

---

### Example 2: Error Handling Complexity

#### SQLite: Simple and Final

**File**: `origin/example-app:examples/prediction-market/src/trading.rs:30-45`
```rust
// All validation happens in one place, errors are final
if balance < total_cost {
    bail!("insufficient balance: need {} cents, have {} cents", total_cost, balance);
}

// Caller just uses ? operator
let trade = app.buy_shares(account_id, market_id, "yes", 100)?;
```

Errors are:
- Constraint violations (insufficient balance, market resolved)
- Database errors (rare with local SQLite)
- Programming bugs

No retry logic needed. No error classification.

#### Message-Passing: Multi-Layer Error Classification

**File**: `examples/prediction-market-bridge/src/bridge/types.rs:54-83`
```rust
// Must track 9 different message stages
pub const STAGE_STATUS_MAP: &[(u8, &str)] = &[
    (0, "not_initialized"),
    (1, "pending"),
    (2, "ready"),
    (3, "pre_execution"),
    (4, "executing"),
    (5, "post_execution"),
    (6, "completed"),
    (7, "failed"),
    (8, "expired"),
];

// Must classify errors as retryable vs permanent
const RETRYABLE_ERRORS: &[&str] = &[
    "NETWORK_ERROR",
    "TIMEOUT",
    "VALIDATOR_UNAVAILABLE",
    "RATE_LIMITED",
    "NONCE_CONFLICT",
    "STORAGE_PUBLISH_FAILED",
    "BRIDGE_SUBMIT_FAILED",
];

pub fn is_retryable_error(error_code: &str) -> bool {
    RETRYABLE_ERRORS.contains(&error_code)
}
```

**File**: `examples/prediction-market-bridge/src/bridge/client.rs:88-127`
```rust
// HTTP submission can fail in many ways
async fn submit_message(&self, message: serde_json::Value) -> PushResult {
    match self.http_client.post(&url).json(&message).send().await {
        Ok(response) => {
            let status = response.status();
            match response.json::<ValidatorResponse>().await {
                Ok(resp) if status.is_success() && resp.status == "accepted" => {
                    PushResult::success(resp.message_id.unwrap_or_default(), resp.signature)
                }
                Ok(resp) => {
                    let error_code = /* extract from response */;
                    PushResult::failure(error_code, error_message, is_retryable_error(error_code))
                }
                Err(e) => PushResult::failure("PARSE_ERROR", &e.to_string(), true)
            }
        }
        Err(e) if e.is_timeout() => PushResult::failure("TIMEOUT", "Request timed out", true),
        Err(e) if e.is_connect() => PushResult::failure("CONNECTION_ERROR", "Failed to connect", true),
        Err(e) => PushResult::failure("HTTP_ERROR", &e.to_string(), true),
    }
}
```

---

### Example 3: Price Storage

#### SQLite: Direct Insert

**File**: `origin/example-app:examples/price-oracle/app/oracle.py:100-130`
```python
def _store_prices(self, prices: list[PriceData]) -> None:
    """Store prices in the database."""
    for price in prices:
        self._conn.execute(
            """
            INSERT INTO prices (asset, source, price, timestamp, volume_24h, market_cap)
            VALUES (?, ?, ?, ?, ?, ?)
            """,
            (price.asset, price.source, price.price, price.timestamp,
             price.volume_24h, price.market_cap),
        )
    self._conn.commit()  # Done. Prices are stored.
```

#### Message-Passing: Submit and Wait

**File**: `examples/price-oracle/src/price_oracle/bridge/client.py:97-178`
```python
async def push_price(self, asset: str, price: float, timestamp: Optional[int] = None,
                     sources_count: Optional[int] = None,
                     price_diff_bps: Optional[int] = None) -> Optional[str]:
    """Push a price update to the Bridge validator."""
    if timestamp is None:
        timestamp = int(time.time())

    price_scaled = int(price * 10**18)  # Scale for on-chain representation
    nonce = self._get_nonce()

    # ABI encode the calldata
    calldata = self._encode_calldata(asset, price_scaled, timestamp)

    # Build message with metadata
    message = {
        "messageType": self.UPDATE_PRICE_SIG,
        "calldata": calldata,
        "metadata": {
            "reason": "price_update",
            "asset": asset,
            "price_scaled": str(price_scaled),
            "timestamp": timestamp,
            "source": "price-oracle",
            "sources_count": sources_count,
            "price_diff_bps": price_diff_bps,
        },
        "nonce": nonce,
        "timestamp": int(time.time()),
        "domain": self.domain,
    }

    return await self._submit_message(message)
    # Note: This only submits. Must still poll for completion!
```

**File**: `examples/price-oracle/src/price_oracle/bridge/client.py:389-443`
```python
async def wait_for_completion(self, message_id: str, timeout: float = 60.0,
                               poll_interval: float = 2.0) -> MessageStatus:
    """Wait for a message to reach a terminal state.

    Polls the validator until the message is completed, failed, or expired,
    or until the timeout is reached.

    This demonstrates the polling pattern required for message-passing:
    - No push notifications: client must poll for status updates
    - Timeout handling: must decide when to give up
    - Multiple terminal states: success, failure, or expiration

    In contrast, SQLite transactions complete synchronously.
    """
    start_time = time.time()
    last_status = None

    while (time.time() - start_time) < timeout:
        status = await self.get_message_status(message_id)

        if last_status is None or status.stage != last_status.stage:
            logger.info(f"Message {message_id[:10]}... stage: {status.stage} ({status.status})")
            last_status = status

        if status.is_terminal:
            return status

        await asyncio.sleep(poll_interval)

    # Timeout reached
    logger.warning(f"Timeout waiting for message {message_id[:10]}...")
    return last_status
```

---

## Summary: Lines of Code by Concern

| Concern | SQLite | Message-Passing |
|---------|--------|-----------------|
| Core business logic | 80 lines | 50 lines (Solidity) |
| Error handling | 5 lines | 80 lines |
| Retry logic | 0 lines | 50 lines |
| Status polling | 0 lines | 45 lines |
| ABI encoding | 0 lines | 50 lines |
| Event sync | 0 lines | 100 lines |
| **Total per operation** | **~85 lines** | **~375 lines** |

The message-passing paradigm requires approximately **4x more code** for the same operation, with most of the additional code handling:
1. Network failures and retries
2. Async status polling
3. Error classification
4. Event synchronization for reads

---

## Recommendation: Use SQLite

**For most applications, use SQLite.** The message-passing paradigm introduces significant complexity that is only justified in narrow circumstances.

### Why SQLite Wins

1. **4x less code** for the same operation
2. **Millisecond latency** vs seconds/minutes
3. **Strong consistency** - no eventual consistency bugs
4. **Simple error handling** - no retry logic, no error classification
5. **Works offline** - no network dependencies
6. **Easier debugging** - state is local and inspectable

### When to Consider Message-Passing

SyndDB's SQLite replication already provides multi-party validation, immutable audit trails, and on-chain guarantees. The only difference with message-passing is **explicit typed messages** vs **implicit messages via table changes**.

| SQLite Replication | Message-Passing |
|--------------------|-----------------|
| `INSERT INTO trades (user, market, shares) VALUES (...)` | `buyShares(market, user, shares)` |
| Implicit intent from row changes | Explicit typed function calls |
| Validators see changesets | Validators see typed messages |

You pay 4x code complexity for that explicitness. The only reason to choose message-passing is if you need typed messages for external integrations or regulatory reasons where table changes aren't self-documenting enough.
