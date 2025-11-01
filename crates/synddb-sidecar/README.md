# SyndDB Sidecar

The SyndDB sidecar monitors SQLite database changes using the Session Extension and publishes them to data availability layers for verification and replication.

## Status

🚧 **Coming Soon** - Implementation in progress

See [PLAN_SIDECAR.md](../../PLAN_SIDECAR.md) for detailed architecture and implementation plan.

## Planned Architecture

1. **Session Monitor** - Attach to SQLite via Session Extension
2. **Batcher** - Accumulate changesets and create periodic snapshots
3. **Attestor** - Compress and sign batches with TEE-protected keys
4. **Publisher** - Publish to multiple DA layers (Celestia, EigenDA, IPFS, Arweave)

## Development Workflow

```bash
# Terminal 1: Run benchmark to generate database activity
cargo run --package synddb-benchmark -- run --rate 100

# Terminal 2: Run sidecar (once implemented)
cargo run --package synddb-sidecar -- --db orderbook.db
```

## Design Goals

- **Zero Code Changes**: Drop-in solution for existing SQLite applications
- **Language Agnostic**: Works with any language that has SQLite bindings
- **High Performance**: Minimal overhead on application performance
- **Deterministic Replication**: Session Extension changesets for validators
- **Automatic Publishing**: Handles all DA layer interaction
