# synddb-chain-monitor

A Rust library for monitoring blockchain events via WebSocket or RPC polling, with built-in idempotency tracking and crash recovery.

## Overview

This crate provides a robust event monitoring system for Ethereum-compatible blockchains. It handles:

- **Event listening** via WebSocket subscriptions or block polling
- **Idempotency tracking** to prevent duplicate event processing
- **Crash recovery** by persisting the last processed block
- **Automatic retries** with configurable timeouts and backoff

## Core Components

### ChainMonitor

The main service that orchestrates blockchain event monitoring. Configure it with contract addresses, block ranges, and RPC endpoints.

### MessageHandler

A trait you implement to process events. The monitor delivers events to your handler, which can decode and process them however needed.

### EventStore

**Important**: The EventStore uses a **separate SQLite database** for internal bookkeeping. This is distinct from your application's database.

The EventStore database tracks:
- Which transaction hashes have been processed (idempotency)
- The last successfully processed block number (crash recovery)

This separation ensures the monitoring infrastructure doesn't interfere with your application data model.

## Quick Start

```rust
use synddb_chain_monitor::{ChainMonitor, ChainMonitorConfig, MessageHandler};
use alloy::{rpc::types::Log, primitives::B256};
use anyhow::Result;

#[derive(Debug)]
struct MyHandler;

#[async_trait::async_trait]
impl MessageHandler for MyHandler {
    async fn handle_event(&self, log: &Log) -> Result<bool> {
        // Process the event and write to YOUR application database
        println!("Event: {:?}", log);
        Ok(true)
    }

    fn event_signature(&self) -> Option<B256> {
        None // Process all events
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = ChainMonitorConfig::new(
        vec![url::Url::parse("wss://eth-mainnet.g.alchemy.com/v2/YOUR_KEY")?],
        "0x1234...".parse()?,
        1000000,
    )
    .with_event_store_path("./chain-monitor-state.db"); // Separate from your app DB!

    let mut monitor = ChainMonitor::new(config, std::sync::Arc::new(MyHandler)).await?;
    monitor.run().await?;
    Ok(())
}
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Your Application                       │
├─────────────────────────────────────────────────────────────┤
│  ChainMonitor                                               │
│  ├─ EthClient (WebSocket/RPC)                              │
│  ├─ MessageHandler (your implementation)                   │
│  └─ EventStore ──> chain-monitor-state.db (internal)       │
│                                                              │
│  Your Handler ──> your-app.db (application data)           │
└─────────────────────────────────────────────────────────────┘
```

**Note**: You'll have two SQLite databases:
1. **EventStore DB** (`chain-monitor-state.db`) - Chain monitor's internal state
2. **Your App DB** (`your-app.db`) - Your application's data

## Documentation

- [Examples](examples/README.md) - Detailed usage examples with different patterns
- [API Documentation](https://docs.rs/synddb-chain-monitor) - Full API reference

## Features

- WebSocket subscriptions with automatic reconnection
- Fallback to block polling when WebSocket unavailable
- Configurable timeouts and retry intervals
- Event signature filtering for specific event types
- Persistent state across restarts
- Production-ready error handling and logging
