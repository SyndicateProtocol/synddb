# Chain Monitor Integration Guide

End-to-end guide for integrating blockchain monitoring into SyndDB applications.

## Architecture

```
┌─────────────────────────────────────────────────┐
│         VM1: Application VM (No Keys)           │
├─────────────────────────────────────────────────┤
│                                                  │
│  SQLite DB ◄──── synddb-client                  │
│    ├─ app tables     ├─ SessionMonitor          │
│    └─ deposits       ├─ ChangesetSender         │
│                      └─ process_deposits()      │
│                              ▲                   │
│                              │ deposits          │
│                              │                   │
│         synddb-chain-monitor │                   │
│           ├─ ChainMonitor    │                   │
│           ├─ DepositHandler  │                   │
│           └─ EventStore      │                   │
│                    │                             │
│                    ▼                             │
│         Base Blockchain                          │
│         Bridge Contract                          │
│                                                  │
│         ────────► synddb-sequencer (VM2) ───►   │
│                   Signs & publishes              │
└─────────────────────────────────────────────────┘
```

**Key Points**:
- Chain monitor runs in VM1 (no signing keys)
- Sequencer in VM2 (isolated, only signs)
- Communication via HTTP (changesets) and channels (deposits)

## Data Flow

### Blockchain → Database

1. ChainMonitor receives events via WebSocket
2. DepositHandler decodes and validates
3. Deposit sent via channel to main thread
4. `process_deposits()` inserts into SQLite
5. SessionMonitor captures INSERT as changeset
6. EventStore prevents duplicate processing

### Application → Sequencer

1. Application writes to SQLite
2. SessionMonitor captures changeset
3. ChangesetSender publishes to sequencer
4. Sequencer signs and publishes to DA layer
5. Replicas apply changesets

## Configuration

### ChainMonitorConfig

Uses clap::Parser for CLI/environment variable configuration:

```rust
// Parse from environment
let chain_config = ChainMonitorConfig::parse();

// Or construct programmatically
let chain_config = ChainMonitorConfig::new(
    vec![Url::parse("wss://base-mainnet.g.alchemy.com/v2/KEY")?],
    "0x1234...".parse()?,
    10_000_000,
)
.with_event_store_path("./chain_events.db");
```

See `crates/synddb-chain-monitor/src/config.rs` for all options.

### SyndDB Client Config

```rust
let config = Config {
    chain_monitor: Some(chain_config),
    ..Default::default()
};
```

See `crates/synddb-client/src/config.rs`.

## Usage

### Basic Integration

```rust
use synddb_client::{Config, SyndDB};

// Configure
let config = Config {
    chain_monitor: Some(chain_monitor_config),
    ..Default::default()
};

// Attach (must have 'static lifetime)
let conn = Box::leak(Box::new(Connection::open("app.db")?));
let synddb = SyndDB::attach_with_config(conn, config)?;

// Process deposits periodically
loop {
    synddb.process_deposits()?;
    // ... application logic ...
    thread::sleep(Duration::from_secs(1));
}
```

See `crates/synddb-client/examples/chain_monitor_example.rs` for complete example.

### Running Examples

```bash
# Configure via environment
WS_URL=wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY \
CONTRACT_ADDRESS=0x... \
START_BLOCK=10000000 \
cargo run --example chain_monitor_example --features chain-monitor

# Or use CLI args
cargo run --example chain_monitor_example --features chain-monitor -- \
  --ws-url wss://... \
  --contract-address 0x... \
  --start-block 10000000
```

## Testing Locally

### 1. Start Local Chain

```bash
anvil
```

### 2. Deploy Contract

```bash
forge create --rpc-url http://127.0.0.1:8545 \
    --private-key <KEY> \
    BridgeContract
```

### 3. Run Sequencer

```bash
cd crates/synddb-sequencer
cargo run --release
```

### 4. Run Client

```bash
WS_URL=ws://127.0.0.1:8545 \
CONTRACT_ADDRESS=<deployed-address> \
START_BLOCK=0 \
cargo run --example chain_monitor_example --features chain-monitor
```

## Production Considerations

### RPC Redundancy

Configure multiple WebSocket URLs for failover:

```rust
ChainMonitorConfig::new(
    vec![
        Url::parse("wss://primary.rpc.com")?,
        Url::parse("wss://backup.rpc.com")?,
        Url::parse("wss://tertiary.rpc.com")?,
    ],
    contract_address,
    start_block,
)
```

### Deposit Processing

Call `process_deposits()` regularly (every 1-5 seconds):

```rust
// Option 1: In application loop
loop {
    synddb.process_deposits()?;
    thread::sleep(Duration::from_secs(1));
}

// Option 2: Dedicated thread
thread::spawn(move || {
    loop {
        synddb.process_deposits().ok();
        thread::sleep(Duration::from_secs(2));
    }
});
```

### Monitoring

```rust
// Check recovery stats
if let Some(stats) = synddb.recovery_stats()? {
    println!("Failed changesets: {}", stats.failed_changesets);
    println!("Failed snapshots: {}", stats.failed_snapshots);
}
```

### Event Store

The chain monitor maintains a separate SQLite database (`chain_events.db`) for idempotency. Consider periodic backups.

## Security

### Two-VM Isolation

**VM1 (Application + Chain Monitor)**:
- ✅ Application code
- ✅ Blockchain monitoring
- ✅ SQLite database
- ❌ NO signing keys

**VM2 (Sequencer)**:
- ✅ Signing keys
- ✅ Transaction signing
- ❌ NO application code
- ❌ NO chain monitoring

This ensures application vulnerabilities cannot leak signing keys.

## Troubleshooting

### Chain Monitor Not Connecting

Check WebSocket URL:
```bash
websocat wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY
```

### Deposits Not Appearing

1. Verify contract address
2. Check start_block is before deposits
3. Verify event signature (if specified)
4. Check logs for errors

### Changesets Not Publishing

1. Verify sequencer is running
2. Check `sequencer_url` in config
3. Check recovery stats for failed batches
4. Review logs

## API Reference

See full documentation:

```bash
cargo doc --features chain-monitor --open
```

## Examples

- **simple_monitor.rs**: Basic event monitoring
- **deposit_handler.rs**: Deposit-specific processing
- **multi_event_handler.rs**: Multiple event types
- **chain_monitor_example.rs**: Full integration

See `crates/synddb-chain-monitor/examples/README.md`.

## Summary

Chain monitor integration enables:

1. ✅ Real-time blockchain event monitoring
2. ✅ Automatic insertion of blockchain data into SQLite
3. ✅ Automatic replication of all changes (app + blockchain)
4. ✅ Strong security with two-VM architecture
5. ✅ Graceful failure handling with idempotency
6. ✅ High availability with RPC failover

---

For implementation details, see the source code in `crates/synddb-chain-monitor/` and `crates/synddb-client/`.
