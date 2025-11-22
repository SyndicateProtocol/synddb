# Chain Monitor Examples

This directory contains examples demonstrating how to use the `synddb-chain-monitor` crate.

## Examples

### 1. Simple Monitor (`simple_monitor.rs`)

The most basic example - monitors all events from a contract and logs them.

**Run:**
```bash
WS_URL=wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY \
CONTRACT_ADDRESS=0x1234567890123456789012345678901234567890 \
START_BLOCK=10000000 \
cargo run --example simple_monitor
```

**What it demonstrates:**
- Basic `MessageHandler` implementation
- Monitoring all events from a contract
- Minimal configuration

### 2. Deposit Handler (`deposit_handler.rs`)

Focused example showing how to handle a specific event type (deposits).

**Run:**
```bash
WS_URL=wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY \
CONTRACT_ADDRESS=0x1234567890123456789012345678901234567890 \
START_BLOCK=10000000 \
cargo run --example deposit_handler
```

**What it demonstrates:**
- Event signature filtering
- Decoding specific event types using Alloy's `sol!` macro
- Event validation
- Handler lifecycle hooks (`on_start`, `on_stop`)
- State tracking across events

### 3. Multi-Event Handler (`multi_event_handler.rs`)

Advanced example showing how to handle multiple event types from a Bridge contract.

**Run:**
```bash
WS_URL=wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY \
CONTRACT_ADDRESS=0x1234567890123456789012345678901234567890 \
START_BLOCK=10000000 \
cargo run --example multi_event_handler
```

**What it demonstrates:**
- Routing different event types to different handlers
- Shared state management across handlers
- Statistics and metrics tracking
- Error handling and recovery
- Production-ready patterns

## Environment Variables

All examples require these environment variables:

- **`WS_URL`**: WebSocket RPC endpoint (e.g., `wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY`)
- **`CONTRACT_ADDRESS`**: Contract address to monitor (e.g., `0x1234...`)
- **`START_BLOCK`**: Block number to start monitoring from (e.g., `10000000`)

## Event Definitions

The examples use event definitions from `src/events.rs`:

```rust
use synddb_chain_monitor::events::{Deposit, Withdrawal, StateSync};
```

These are example events for a typical Bridge contract. In your application, you'll define your own events using Alloy's `sol!` macro:

```rust
use alloy::sol;

sol! {
    event MyEvent(address indexed user, uint256 amount);
}
```

## Testing Examples

For testing without a real blockchain connection, you can:

1. Use a local testnet (e.g., Anvil, Hardhat)
2. Use a public testnet (e.g., Base Sepolia)
3. Mock the RPC responses (for unit tests)

**Example with local Anvil:**

```bash
# Terminal 1: Start Anvil
anvil

# Terminal 2: Deploy contract and run monitor
WS_URL=ws://127.0.0.1:8545 \
CONTRACT_ADDRESS=0x5FbDB2315678afecb367f032d93F642f64180aa3 \
START_BLOCK=0 \
cargo run --example simple_monitor
```

## Production Considerations

When deploying in production:

1. **Use multiple RPC endpoints** for redundancy:
   ```rust,no_run
   let config = ChainMonitorConfig::new(
       vec![
           Url::parse("wss://primary.rpc.com")?,
           Url::parse("wss://backup.rpc.com")?,
       ],
       contract_address,
       start_block,
   );
   ```

2. **Configure appropriate timeouts**:
   ```rust,no_run
   config
       .with_request_timeout(Duration::from_secs(30))
       .with_get_logs_timeout(Duration::from_secs(600))
   ```

3. **Use persistent event storage**:
   ```rust,no_run
   config.with_event_store_path("/data/events.db")
   ```

4. **Implement proper error handling** in your `MessageHandler`

5. **Add metrics and monitoring** to track event processing

6. **Use structured logging** with appropriate log levels

## Next Steps

- Review the [main documentation](/README.md)
- Explore the [API documentation](https://docs.rs/synddb-chain-monitor)
- Integrate with your application
- Add custom event definitions for your contracts
