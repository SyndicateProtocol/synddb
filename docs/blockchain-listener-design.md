# SyndDB Blockchain Listener: Design & Implementation

## Overview

The `synddb-chain-monitor` crate provides generic blockchain event monitoring for Ethereum-compatible chains. It integrates with `synddb-client` as an optional feature to monitor Bridge contracts and process deposits.

**Critical Architecture**: The sequencer runs in isolation in its own Confidential Space VM and only signs transactions. All blockchain listening runs in a separate VM alongside the client, ensuring key extraction is impossible even if compromised.

## Architecture

### Two-VM Security Model

```
VM1 (Application + Chain Monitor)          VM2 (Sequencer)
├─ synddb-client                           ├─ synddb-sequencer
├─ synddb-chain-monitor                    │  - Signs transactions
├─ Application SQLite DB                   │  - 🔐 Signing keys (isolated)
└─ NO signing keys                         └─ Publishes to storage layer
```

**Security Properties**:
- ✅ VM1 has ZERO access to signing keys
- ✅ VM2 has MINIMAL attack surface (HTTP API only)
- ✅ Chain monitor failure cannot affect signing
- ✅ Blockchain listening in untrusted VM1

### Component Structure

```
crates/
├── synddb-chain-monitor/        # Generic blockchain listener
│   ├── src/
│   │   ├── config.rs           # ChainMonitorConfig (uses clap::Parser)
│   │   ├── eth_client.rs       # WebSocket + RPC client
│   │   ├── monitor.rs          # ChainMonitor service
│   │   ├── handler.rs          # MessageHandler trait
│   │   ├── event_store.rs      # SQLite idempotency tracking
│   │   └── events.rs           # Example Bridge events
│   └── examples/               # See examples/README.md
│
├── synddb-client/               # Integration
│   ├── src/
│   │   ├── chain_handler.rs    # DepositHandler implementation
│   │   ├── chain_monitor_integration/  # Feature-gated module
│   │   └── config.rs           # Config with optional ChainMonitorConfig
│   └── examples/
│       └── chain_monitor_example.rs
```

## Core Concepts

### 1. MessageHandler Trait

Generic trait for processing blockchain events. Applications implement this to handle specific event types.

```rust
#[async_trait::async_trait]
pub trait MessageHandler: Send + Sync + Debug {
    async fn handle_event(&self, log: &Log) -> Result<bool>;
    fn event_signature(&self) -> Option<B256>;
}
```

See `crates/synddb-chain-monitor/src/handler.rs` for details.

### 2. Configuration

Uses clap::Parser for CLI/environment variable support:

```rust,norun
// Parse from environment variables or CLI args
let config = ChainMonitorConfig::parse();

// Or construct programmatically
let config = ChainMonitorConfig::new(
    vec![ws_url],
    contract_address,
    start_block,
)
.with_event_store_path("./chain_events.db");
```

See `crates/synddb-chain-monitor/src/config.rs` and `crates/synddb-client/src/config.rs`.

### 3. Event Processing Flow

1. ChainMonitor detects event via WebSocket/RPC
2. DepositHandler decodes and validates event
3. Event sent via channel to main thread
4. Application calls `process_deposits()` to insert into SQLite
5. SessionMonitor captures INSERT as changeset
6. EventStore tracks processed events (idempotency)

### 4. Integration with synddb-client

The chain monitor is integrated via the `chain-monitor` feature flag:

```rust
// Enable in Cargo.toml
synddb-client = { features = ["chain-monitor"] }

// Configure and attach
let config = Config {
    chain_monitor: Some(chain_monitor_config),
    ..Default::default()
};
let synddb = SyndDB::attach_with_config(conn, config)?;

// Process deposits periodically
synddb.process_deposits()?;
```

See `crates/synddb-client/examples/chain_monitor_example.rs` for complete example.

## Implementation Details

### Dual-Mode Operation

- **WebSocket**: Real-time event subscriptions (preferred)
- **RPC Polling**: Fallback when WebSocket unavailable
- **Auto-detect**: ChainMonitor automatically chooses best mode

### Idempotency & Recovery

- **EventStore**: Separate SQLite database tracks processed tx hashes
- **Crash Recovery**: Resumes from last processed block
- **Deduplication**: Prevents duplicate event processing

### Channel-Based Communication

Chain monitor runs in the same process as the application. Uses crossbeam channels instead of HTTP for efficiency.

## Usage

### Running Examples

```bash
# Configure via environment variables
WS_URL=wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY \
CONTRACT_ADDRESS=0x... \
START_BLOCK=10000000 \
cargo run --example simple_monitor

# Or use clap CLI args
cargo run --example simple_monitor -- \
  --ws-url wss://... \
  --contract-address 0x... \
  --start-block 10000000
```

See `crates/synddb-chain-monitor/examples/README.md`.

### Integration

See `docs/chain-monitor-integration.md` for end-to-end integration guide.

## Testing

**synddb-chain-monitor**: 21 unit tests ✅

Run tests:
```bash
cargo test --package synddb-chain-monitor
cargo test --package synddb-client --features chain-monitor
```

## Documentation

- **API Docs**: `cargo doc --features chain-monitor --open`
- **Examples**: `crates/synddb-chain-monitor/examples/README.md`
- **Integration**: `docs/chain-monitor-integration.md`
- **Source Code**: See actual code for latest implementation details

## Design Decisions

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| Architecture | Trait-based MessageHandler | Generic, supports any event type |
| Communication | Channels (not HTTP) | Same-VM, simpler, faster |
| Configuration | clap::Parser | CLI + env var support |
| Feature Flag | Optional chain-monitor | Smaller binary if not needed |
| Module Structure | Feature-gated modules | Eliminates scattered #[cfg] annotations |

## Future Enhancements

- Graceful shutdown signaling
- Prometheus metrics
- Chain reorganization handling
- Multi-contract monitoring
- Event replay from specific blocks

---

**Status**: ✅ Implementation Complete
**Test Coverage**: 21 unit tests passing
**Documentation**: See source code for implementation details
