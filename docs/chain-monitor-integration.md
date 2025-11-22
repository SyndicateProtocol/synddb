# Chain Monitor Integration - End-to-End Architecture

This document describes the complete integration of blockchain monitoring into SyndDB, showing how all components work together.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                          VM1: Application VM (No Keys)                   │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                           │
│  ┌────────────────┐          ┌────────────────────────────────────────┐ │
│  │   SQLite DB    │◄─────────┤         synddb-client                  │ │
│  │                │          │  - SessionMonitor (changeset capture)  │ │
│  │ ┌────────────┐ │          │  - ChangesetSender (publish to seq)   │ │
│  │ │ App Tables │ │          │  - SnapshotSender (snapshots)          │ │
│  │ │  deposits  │ │          │  - process_deposits() (DB insertion)   │ │
│  │ └────────────┘ │          └────────────────────────────────────────┘ │
│  └────────┬───────┘                       ▲                              │
│           │                               │                              │
│           │ changesets                    │ deposit events               │
│           │                               │                              │
│           │          ┌────────────────────┴─────────────────┐            │
│           │          │    synddb-chain-monitor              │            │
│           │          │  - EthClient (WebSocket/RPC)         │            │
│           │          │  - ChainMonitor (event processor)    │            │
│           │          │  - DepositHandler (event decoder)    │            │
│           │          │  - EventStore (idempotency)          │            │
│           │          └──────────────┬───────────────────────┘            │
│           │                         │                                    │
│           │                         │ WebSocket                          │
│           │                         ▼                                    │
│           │              ┌──────────────────────┐                        │
│           │              │   Base Blockchain     │                       │
│           │              │  Bridge Contract      │                       │
│           │              └──────────────────────┘                        │
│           │                                                               │
│           │ HTTP (changesets + snapshots)                                │
│           ▼                                                               │
├───────────┼───────────────────────────────────────────────────────────────┤
│           │                VM2: Sequencer VM (Has Keys)                  │
│           │                                                               │
│           │          ┌────────────────────────────────────────┐           │
│           └─────────►│       synddb-sequencer                 │           │
│                      │  - Receives changesets                 │           │
│                      │  - Orders & signs transactions         │           │
│                      │  - Publishes to blockchain             │           │
│                      └────────────────────────────────────────┘           │
└───────────────────────────────────────────────────────────────────────────┘
```

## Data Flow

### 1. Blockchain Event → Database

1. **Event Detection**: Chain monitor subscribes to blockchain events via WebSocket
2. **Event Decoding**: `DepositHandler` decodes Deposit events using Alloy's type-safe sol! macros
3. **Validation**: Handler validates deposit amount, checks for zero amounts
4. **Channel Send**: Valid deposits sent via crossbeam channel to main thread
5. **Database Insertion**: Main thread calls `process_deposits()` to insert into SQLite
6. **Changeset Capture**: SessionMonitor automatically captures INSERT as a changeset
7. **Idempotency**: EventStore tracks processed events to prevent duplicates

### 2. Application Change → Sequencer

1. **Application Write**: App performs `INSERT/UPDATE/DELETE` on SQLite
2. **Session Hooks**: SQLite session extension captures the changeset
3. **Buffering**: SessionMonitor buffers changesets
4. **Periodic Publish**: Every `publish_interval` (default 1s), send changesets to sequencer
5. **Sequencer Processing**: Sequencer orders, signs, and publishes to blockchain
6. **Replication**: Other replicas apply the same changesets

## Components

### synddb-chain-monitor (`crates/synddb-chain-monitor/`)

**Purpose**: Generic blockchain event monitoring library

**Key Files**:
- `src/lib.rs` - Public API and re-exports
- `src/config.rs` - ChainMonitorConfig with WebSocket URLs, contract address, etc.
- `src/eth_client.rs` - Robust WebSocket client with auto-reconnect
- `src/monitor.rs` - ChainMonitor service (WebSocket or RPC polling)
- `src/handler.rs` - MessageHandler trait for custom event processing
- `src/event_store.rs` - SQLite-based idempotency tracking
- `src/events.rs` - Example Bridge contract events (Deposit, Withdrawal, etc.)

**Features**:
- Dual-mode operation: WebSocket (preferred) or RPC polling (fallback)
- Automatic failover between multiple RPC URLs
- Event idempotency with SQLite tracking
- Crash recovery (resumes from last processed block)
- Generic trait-based architecture (works with any event type)

### synddb-client (`crates/synddb-client/`)

**Purpose**: Application-side SQLite session wrapper with blockchain integration

**Key Files** (new/modified for chain monitor):
- `src/config.rs` - Added `ChainMonitorConfig` option
- `src/chain_handler.rs` - `DepositHandler` implementation
- `src/lib.rs` - Integration of chain monitor into SyndDB lifecycle
  - `start_chain_monitor()` - Spawns chain monitor thread
  - `process_deposits()` - Public method to insert pending deposits
  - `attach_with_config()` - Updated to optionally start chain monitor

**Feature Flag**: `chain-monitor` (optional)
```toml
synddb-client = { features = ["chain-monitor"] }
```

**New API**:
```rust,no_run
// Process pending deposits (call periodically from main thread)
let count = synddb.process_deposits()?;
```

### Integration Points

1. **Configuration**: `Config.chain_monitor` option
2. **Lifecycle**: Chain monitor thread spawned in `attach_with_config()`
3. **Channel**: Deposit events sent via channel to main thread
4. **Database**: `process_deposits()` inserts deposits into SQLite
5. **Automatic Capture**: SessionMonitor captures deposit INSERTs as changesets

## Running End-to-End

### Prerequisites

1. **RPC Access**: Alchemy, Infura, or self-hosted Base node with WebSocket support
2. **Contract**: Deployed Bridge contract address
3. **Sequencer**: Running synddb-sequencer instance

### Step 1: Configure the Client

```rust,no_run
use synddb_client::{Config, SyndDB};

let config = Config {
    sequencer_url: "http://sequencer:8433".to_string(),
    publish_interval: Duration::from_secs(1),
    chain_monitor: Some(synddb_client::config::ChainMonitorConfig {
        ws_urls: vec![
            "wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY".to_string(),
            "wss://backup-rpc.example.com".to_string(), // Redundancy
        ],
        contract_address: "0x1234567890123456789012345678901234567890".to_string(),
        start_block: 10_000_000, // Start from this block
        event_signature: None, // Or specify Deposit::SIGNATURE_HASH for filtering
        event_store_path: "./chain_events.db".to_string(),
        deposit_table: "deposits".to_string(),
    }),
    ..Default::default()
};
```

### Step 2: Attach and Run

```rust,no_run
// Create SQLite connection (must be 'static)
let conn = Box::leak(Box::new(Connection::open("app.db")?));

// Attach SyndDB
let synddb = SyndDB::attach_with_config(conn, config)?;

// Main application loop
loop {
    // 1. Process blockchain deposits
    synddb.process_deposits()?;

    // 2. Perform application operations
    conn.execute(
        "INSERT INTO trades VALUES (?1, ?2)",
        params![trade_id, amount],
    )?;

    // 3. Manually publish if needed (or let auto-publish handle it)
    // synddb.publish()?;

    // 4. Sleep/wait for next iteration
    thread::sleep(Duration::from_secs(1));
}
```

### Step 3: Run the Example

```bash
# Set environment variables
export WS_URL="wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY"
export CONTRACT_ADDRESS="0x1234567890123456789012345678901234567890"
export START_BLOCK="10000000"

# Run the example
cargo run --example chain_monitor_example --features chain-monitor
```

## Testing Locally

### 1. Start Local Blockchain

```bash
# Terminal 1: Run Anvil (local Ethereum node)
anvil
```

### 2. Deploy Test Contract

```bash
# Terminal 2: Deploy a test Bridge contract
forge create --rpc-url http://127.0.0.1:8545 \
    --private-key <KEY> \
    BridgeContract
```

### 3. Run Sequencer

```bash
# Terminal 3: Start sequencer
cd crates/synddb-sequencer
cargo run --release
```

### 4. Run Client

```bash
# Terminal 4: Run client with local chain
export WS_URL="ws://127.0.0.1:8545"
export CONTRACT_ADDRESS="<deployed-contract-address>"
export START_BLOCK="0"

cargo run --example chain_monitor_example --features chain-monitor
```

## Production Considerations

### 1. RPC Redundancy

Always configure multiple WebSocket URLs for failover:

```rust,no_run
ws_urls: vec![
    "wss://primary-rpc.com".to_string(),
    "wss://backup-rpc.com".to_string(),
    "wss://tertiary-rpc.com".to_string(),
],
```

### 2. Deposit Processing

Call `process_deposits()` regularly (every 1-5 seconds) to ensure deposits are inserted promptly:

```rust,no_run
// Option 1: In application event loop
loop {
    synddb.process_deposits()?;
    // ... application logic ...
    thread::sleep(Duration::from_secs(1));
}

// Option 2: Dedicated deposit processing thread
thread::spawn(move || {
    loop {
        if let Err(e) = synddb.process_deposits() {
            error!("Deposit processing error: {}", e);
        }
        thread::sleep(Duration::from_secs(2));
    }
});
```

### 3. Error Handling

Chain monitor runs indefinitely. Monitor logs for connection issues:

```
INFO  Chain monitor started successfully
INFO  Deposits table 'deposits' ready
INFO  Using WebSocket subscription for real-time events
INFO  Deposit queued for database insertion
```

### 4. Event Store Maintenance

The chain monitor maintains a SQLite database (`chain_events.db`) for idempotency. Consider periodic backups:

```bash
# Backup event store
cp ./chain_events.db ./chain_events_backup.db
```

### 5. Monitoring

Track key metrics:
- Deposits processed per minute
- Chain monitor connection status
- Changeset publish success rate
- Recovery stats (failed batches)

```rust,no_run
// Check recovery stats
if let Some(stats) = synddb.recovery_stats()? {
    println!("Failed changesets: {}", stats.failed_changesets);
    println!("Failed snapshots: {}", stats.failed_snapshots);
}
```

## Security Considerations

### Two-VM Architecture

**VM1 (Application + Chain Monitor)**:
- ✅ Runs application code
- ✅ Monitors blockchain
- ✅ Has SQLite database
- ❌ NO signing keys
- ❌ Cannot authorize blockchain transactions

**VM2 (Sequencer)**:
- ✅ Has signing keys
- ✅ Signs and publishes transactions
- ❌ NO application code
- ❌ NO chain monitoring
- ❌ Only receives changesets via HTTP

This separation ensures:
1. Application vulnerabilities cannot leak signing keys
2. Sequencer is isolated and only does transaction signing
3. Chain monitor compromise cannot authorize transactions

## Troubleshooting

### Chain Monitor Not Connecting

Check WebSocket URL and network connectivity:
```bash
websocat wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY
```

### Deposits Not Appearing

1. Check contract address is correct
2. Verify start_block is before actual deposits
3. Check event signature matches (if specified)
4. Look for errors in logs

### Changesets Not Publishing

1. Check sequencer is running and accessible
2. Verify `sequencer_url` in config
3. Check recovery stats for failed batches
4. Ensure `publish_interval` is reasonable

## Future Enhancements

Potential improvements for production:

1. **Graceful Shutdown**: Add shutdown signal handling for chain monitor
2. **Metrics Export**: Prometheus metrics for monitoring
3. **Configurable Retries**: More control over RPC retry behavior
4. **Event Filtering**: More sophisticated event filtering options
5. **Multi-Contract Support**: Monitor multiple contracts simultaneously
6. **State Sync**: Initial state sync for new replicas

## Summary

The chain monitor integration enables SyndDB to:

1. ✅ Listen to blockchain events (deposits) in real-time
2. ✅ Insert blockchain data into local SQLite database
3. ✅ Automatically replicate both app changes and blockchain events
4. ✅ Maintain strong security with two-VM architecture
5. ✅ Handle failures gracefully with idempotency and crash recovery
6. ✅ Scale with multiple RPC endpoints and automatic failover

All components work together seamlessly to create a unified view of both application state and blockchain state, automatically replicated across all nodes in the SyndDB network.
