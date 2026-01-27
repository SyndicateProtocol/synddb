# SyndDB Code Architecture

This document describes the code organization, patterns, and conventions used in the SyndDB codebase. It's intended for contributors and third-party developers integrating with or extending SyndDB.

For high-level system architecture, see [SPEC.md](SPEC.md).

## Crate Organization

```
crates/
├── synddb-shared/        # Shared types and utilities (no business logic)
├── synddb-client/        # Client library for applications
├── synddb-sequencer/     # Message sequencing and publishing service
├── synddb-validator/     # State validation and replica reconstruction
├── synddb-chain-monitor/ # Blockchain event monitoring
└── synddb-benchmark/     # Performance testing tools
```

### Dependency Flow

```
synddb-shared (foundation)
     ↑
     ├── synddb-client
     ├── synddb-sequencer
     ├── synddb-validator
     └── synddb-chain-monitor
```

All crates depend on `synddb-shared` for common types. Crates do not depend on each other horizontally.

## Key Shared Types

### `synddb-shared/src/types/`

| Module | Purpose |
|--------|---------|
| `batch.rs` | `BatchInfo` struct and filename utilities for batch storage |
| `cbor/` | CBOR/COSE binary wire format (primary serialization) |
| `message.rs` | `SignedMessage`, `SignedBatch` after parsing from CBOR |
| `payloads.rs` | HTTP request/response payload types |

### `synddb-shared/src/gcs.rs`

Shared GCS configuration used by both sequencer and validator:

```rust
use synddb_shared::gcs::GcsConfig;

let config = GcsConfig::new("my-bucket")
    .with_prefix("sequencer/v1")
    .with_emulator_host("http://localhost:4443");  // for testing
```

## Configuration Patterns

All configurations use a consistent pattern combining clap for CLI args and serde for file/env config:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
pub struct Config {
    #[arg(long, env = "BIND_ADDRESS", default_value = "0.0.0.0:8080")]
    pub bind_address: SocketAddr,

    #[arg(long, env = "TIMEOUT", value_parser = parse_duration)]
    #[serde(with = "humantime_serde")]
    pub timeout: Duration,
}
```

### Builder Methods for Testing

All config structs provide builder methods for test configuration:

```rust
let config = SequencerConfig::with_signing_key("0x...")
    .with_bind_address("127.0.0.1:0".parse().unwrap())
    .with_publisher_type(PublisherType::Local)
    .with_batch_config(100, 1_000_000);
```

## Error Handling Philosophy

The codebase uses a layered approach:

| Layer | Approach | Example |
|-------|----------|---------|
| Domain boundaries (HTTP API) | Custom `thiserror` enums with status codes | `SequencerError`, `ValidatorError` |
| Internal operations | `anyhow::Result<T>` with `.context()` | Most internal functions |
| Infallible-in-practice | `expect()` with descriptive message | Signal handlers, in-memory compression |

### HTTP Error Example

```rust
#[derive(Debug, Error)]
pub enum SequencerError {
    #[error("Message not found: sequence {0}")]
    MessageNotFound(u64),
    // ...
}

impl From<SequencerError> for HttpError {
    fn from(err: SequencerError) -> Self {
        match err {
            SequencerError::MessageNotFound(_) => (StatusCode::NOT_FOUND, ...),
            // ...
        }
    }
}
```

## Transport/Storage Abstractions

### Sequencer: `TransportPublisher` trait

For publishing batches to storage backends (GCS, local, future: Arweave):

```rust
#[async_trait]
pub trait TransportPublisher: Send + Sync + Debug {
    fn name(&self) -> &str;
    async fn publish(&self, batch: &CborBatch) -> Result<PublishMetadata, TransportError>;
    async fn fetch(&self, start_sequence: u64) -> Result<Option<CborBatch>, TransportError>;
    async fn list_batches(&self) -> Result<Vec<BatchInfo>, TransportError>;
    async fn get_latest_sequence(&self) -> Result<Option<u64>, TransportError>;
    async fn get_message(&self, sequence: u64) -> Result<Option<CborSignedMessage>, TransportError>;
}
```

### Validator: `StorageFetcher` trait

For fetching batches from storage (read-only counterpart):

```rust
#[async_trait]
pub trait StorageFetcher: Send + Sync + Debug {
    fn name(&self) -> &str;
    fn supports_batches(&self) -> bool;
    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>>;
    async fn get_latest_sequence(&self) -> Result<Option<u64>>;
    async fn list_batches(&self) -> Result<Vec<BatchInfo>>;
    async fn get_batch(&self, start_sequence: u64) -> Result<Option<SignedBatch>>;
    async fn get_batch_by_path(&self, path: &str) -> Result<Option<SignedBatch>>;
}
```

## Adding New Features

### Adding a New Publisher (Sequencer)

1. Create `crates/synddb-sequencer/src/transport/newbackend.rs`
2. Implement `TransportPublisher` trait
3. Add feature flag to `Cargo.toml`
4. Wire up in `main.rs` with feature gate

### Adding a New Fetcher (Validator)

1. Create `crates/synddb-validator/src/sync/providers/newbackend.rs`
2. Implement `StorageFetcher` trait
3. Add feature flag if needed
4. Add to `FetcherType` enum in config
5. Wire up in fetcher creation logic

### Adding a New HTTP Endpoint

1. Add request/response types in `http_api.rs`
2. Add handler function
3. Register route in `create_router()`
4. Add tests

## Testing Patterns

### Unit Tests

Inline in source files using `#[cfg(test)] mod tests`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature() {
        let config = Config::with_signing_key("0x...".repeat(32));
        // ...
    }
}
```

### Integration Tests

Use in-memory SQLite and builder configs:

```rust
#[tokio::test]
async fn test_sequencer_flow() {
    let config = SequencerConfig::with_signing_key(TEST_KEY)
        .with_publisher_type(PublisherType::Local);
    // ...
}
```

## Async Patterns

- **Runtime**: Tokio with feature flags based on use case
- **Sync-async boundary**: `crossbeam-channel` for thread communication
- **Traits**: `#[async_trait]` for async trait methods
- **Shutdown**: `tokio::sync::watch` channels for graceful shutdown

## Logging

Use `tracing` macros with structured fields:

```rust
use tracing::{info, warn, debug, error};

info!(sequence = seq, batch_size = batch.len(), "Published batch");
warn!(error = %e, "Retrying operation");
```

## Code Style

- No emojis unless explicitly requested
- Prefer editing existing files over creating new ones
- Keep solutions minimal - avoid over-engineering
- Use descriptive variable names
- Add doc comments for public APIs

## CI Validation

Before committing, run the full CI suite locally:

```bash
cargo +nightly fmt --all
cargo clippy --workspace --all-targets --all-features
cargo machete
taplo fmt "**/Cargo.toml"
cargo test --workspace
```

See [CLAUDE.md](CLAUDE.md) for detailed development instructions.
