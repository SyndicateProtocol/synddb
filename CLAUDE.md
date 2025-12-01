# CLAUDE.md

This is the guide for Claude Code when working with the SyndDB monorepo.

## Project Overview

SyndDB is a SQLite replication system for blockchain-based applications. It captures SQLite changesets, sequences them through a trusted sequencer, and publishes to data availability layers for deterministic state reconstruction.

**Architecture**: Two-VM security model where the application (VM1) captures changes and the sequencer (VM2) signs them with isolated keys.

## Crate Structure

```
crates/
├── synddb-client/        # Client library for changeset capture (Rust + FFI)
├── synddb-sequencer/     # Message ordering and signing service
├── synddb-chain-monitor/ # Blockchain event monitoring (WebSocket/RPC)
└── synddb-benchmark/     # Orderbook simulator for performance testing
```

## Development Patterns

### Configuration
All configs derive `Serialize`, `Deserialize`, and clap `Parser`. Support both CLI args and env vars:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
pub struct Config {
    #[arg(long, env = "VAR_NAME", default_value = "value")]
    pub field: Type,

    #[arg(long, env = "DURATION", value_parser = parse_duration)]
    #[serde(with = "humantime_serde")]
    pub timeout: Duration,
}
```

### Error Handling
- `anyhow::Result<T>` for fallible operations
- `thiserror::Error` for custom error types
- Add `.context()` at system boundaries

### Async Patterns
- Tokio runtime with `current_thread` for background tasks
- `crossbeam-channel` for sync-async boundaries
- `async_trait` for async trait methods

### Testing
- Inline `#[cfg(test)] mod tests` in source files
- Builder methods for test configs: `Config::with_signing_key()`
- In-memory SQLite: `Connection::open_in_memory()`
- Don't import libraries for specific tests, import them at the top level instead

### Project structure
- Do not re-export dependencies from `crates/` in mod.rs or lib.rs files. Refer to the full path instead.

## Key Dependencies

| Category | Crates |
|----------|--------|
| Async | `tokio`, `async-trait` |
| Web | `axum`, `reqwest`, `tower` |
| Serialization | `serde`, `serde_json`, `humantime-serde` |
| Blockchain | `alloy` (provider-ws, rpc-types, sol-types, signers) |
| Database | `rusqlite` (bundled, session, hooks) |
| CLI | `clap` (derive) |
| Errors | `anyhow`, `thiserror` |
| Logging | `tracing`, `tracing-subscriber` |

## Feature Flags

- `synddb-client`: `chain-monitor`, `ffi`
- `synddb-sequencer`: `gcs`, `tee`, `celestia`, `eigenda`

## CI Validation

Before committing, run the full CI validation locally:

```bash
# Full CI check (format, lint, unused deps, TOML format)
cargo +nightly fmt --all -- --unstable-features && \
cargo clippy --workspace --all-targets --all-features && \
cargo machete && \
taplo fmt "**/Cargo.toml"

# Run tests
cargo test --workspace

# Build all features
cargo build --workspace --all-features

# Check specific crate
cargo clippy -p synddb-sequencer --all-features
cargo test -p synddb-sequencer
```

**Required tools:**
- `rustup install nightly` (for fmt)
- `cargo install taplo-cli` (TOML formatting)
- `cargo install cargo-machete` (unused dependency detection)

## Code Style

- No emojis in code or comments unless explicitly requested
- Prefer editing existing files over creating new ones
- Keep solutions minimal - avoid over-engineering
- Use `tracing` macros (`info!`, `warn!`, `debug!`, `error!`) for logging
- Structured logging with fields: `info!(sequence = seq, "Message")`
- Do not add self-explanatory comments that do not clarify code. Below is an example of useless comments:

```rust
// Create shutdown channel
let (shutdown_tx, shutdown_rx) = watch::channel(false);

// Create DA fetcher based on config
let fetcher = create_fetcher(&config).await?;

// Create validator
let mut validator = Validator::new(&config, fetcher.clone(), shutdown_rx.clone())?;
```

## Common Workflows

### Adding a new endpoint (synddb-sequencer)
1. Add request/response types in `http_api.rs`
2. Add handler function
3. Register route in `create_router()`
4. Add tests

### Adding a new publisher (synddb-sequencer)
1. Implement `DAPublisher` trait in `publish/`
2. Add feature flag to `Cargo.toml`
3. Wire up in `main.rs` with feature gate

### Adding chain monitor handler (synddb-client)
1. Implement `MessageHandler` trait
2. Register with `ChainMonitor::add_handler()`
