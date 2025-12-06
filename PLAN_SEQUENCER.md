# PLAN_SEQUENCER.md - Sequencer Service for Publishing to DA Layers

## Overview

The synddb-sequencer is a service that receives changesets and snapshots from application client libraries and publishes them to multiple DA layers.

**Architecture Note**: The original plan described the sequencer as a sidecar process that directly monitors SQLite databases. We have since evolved to a **client library architecture**:

- **Client Library** (`synddb-client` crate): Embeds in applications, captures changesets/snapshots via SQLite Session Extension, sends to sequencer via HTTP
- **Sequencer Service** (`synddb-sequencer` crate - this document): Receives from client libraries, publishes to DA layers, monitors blockchain for inbound messages
- **Security**: Client and sequencer run in **separate TEEs** to isolate signing keys from the application

This document focuses on the **sequencer service** implementation. For client library details, see `crates/synddb-client/`.

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│             Application + SyndDB Client Library              │
│         (in TEE #1 - Application TEE)                        │
│                                                              │
│  ┌────────────────────────────────────────────────────┐     │
│  │  Application → SQLite + Session Extension          │     │
│  │  (Python/JS/Go/Rust/Java)                          │     │
│  └────────────────────────────────────────────────────┘     │
│                          ↓                                   │
│  ┌────────────────────────────────────────────────────┐     │
│  │  SyndDB Client Library (synddb-client crate)       │     │
│  │  - Captures changesets via Session Extension       │     │
│  │  - Creates snapshots (periodic + on schema change) │     │
│  │  - Includes TEE attestation tokens                 │     │
│  └────────────────────────────────────────────────────┘     │
└──────────────────────────────────────────────────────────────┘
                          ↓ HTTP POST
                    (Changesets + Snapshots)
                          ↓
┌──────────────────────────────────────────────────────────────┐
│               synddb-sequencer (in TEE #2)                   │
│                                                              │
│ ┌──────────────────────────────────────────────────┐        │
│ │         HTTP Receiver (Axum)                     │        │
│ │  - Receives changesets/snapshots from clients    │        │
│ │  - Verifies TEE attestation tokens               │        │
│ └──────────────────────────────────────────────────┘        │
│                          ↓                                   │
│ ┌──────────────────────────────────────────────────┐        │
│ │         Batcher + Attestor                       │        │
│ │  - Batches operations                            │        │
│ │  - Compresses (zstd)                             │        │
│ │  - Signs with sequencer keys                     │        │
│ └──────────────────────────────────────────────────┘        │
│                          ↓                                   │
│ ┌──────────────────────────────────────────────────┐        │
│ │         Multi-DA Publisher                       │        │
│ │  ┌─────────┐ ┌─────────┐ ┌──────┐ ┌─────────┐  │        │
│ │  │Celestia │ │EigenDA  │ │ IPFS │ │ Arweave │  │        │
│ │  └─────────┘ └─────────┘ └──────┘ └─────────┘  │        │
│ └──────────────────────────────────────────────────┘        │
│                                                              │
│ ┌──────────────────────────────────────────────────┐        │
│ │         Deposit Monitor                          │        │
│ │  ┌────────────────┐    ┌────────────────────┐   │        │
│ │  │ Chain Monitor  │───▶│  Deposit HTTP API  │   │        │
│ │  │ (Bridge Events)│    │  (to Applications) │   │        │
│ │  └────────────────┘    └────────────────────┘   │        │
│ └──────────────────────────────────────────────────┘        │
└──────────────────────────────────────────────────────────────┘
                          ↑
                    Event Monitoring
                          │
┌──────────────────────────────────────────────────────────────┐
│                    Blockchain (L1/L2)                        │
│                 Bridge Contract (Deposits)                   │
└──────────────────────────────────────────────────────────────┘

Key Changes from Original Plan:
- Client library handles Session Extension monitoring (already implemented in `synddb-client`)
- Sequencer receives via HTTP instead of direct database access
- Two separate TEEs for security: application + client vs sequencer
- Signing keys isolated in sequencer TEE
```

## Core Libraries

```toml
[dependencies]
# Note: SQLite Session Extension monitoring is handled by synddb-client library
# The sequencer receives pre-captured changesets/snapshots via HTTP

# Compression and serialization
zstd = "0.13"  # Fast compression with good ratios
bincode = "1.3"  # Efficient binary serialization
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# DA Layer clients
celestia-client = "0.2"  # Celestia DA
eigenda-rust = "0.1"  # EigenDA client
ipfs-api = "0.11"  # IPFS client
arweave-rs = "0.1"  # Arweave client

# Async runtime and networking
tokio = { version = "1.35", features = ["full"] }
hyper = { version = "1.0", features = ["full"] }
tower = "0.4"  # Middleware stack for retries/timeouts
axum = { version = "0.7", features = ["sse"] }  # HTTP server with SSE support

# Blockchain monitoring for deposits
alloy = { version = "0.1", features = ["full"] }  # Modern Ethereum client from Foundry team
alloy-rpc-client = "0.1"  # RPC client for blockchain interaction
alloy-contract = "0.1"  # Contract interaction utilities

# GCP Confidential Space and Ethereum signing
gcp-auth = "0.10"  # GCP authentication
google-cloud-secretmanager = "0.6"  # Secret Manager for key storage
google-cloud-default = "0.6"  # Workload Identity support
k256 = { version = "0.13", features = ["ecdsa", "sha256"] }  # secp256k1 for Ethereum
sha3 = "0.10"  # Keccak256 for Ethereum address derivation
reqwest = { version = "0.11", features = ["json"] }  # Attestation token fetching
hex = "0.4"  # Hex encoding for addresses

# Configuration and monitoring
config = "0.14"  # YAML/TOML configuration
google-cloud-logging = "0.6"  # GCP Cloud Logging integration
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
sysinfo = "0.30"  # System monitoring

# Error handling and utilities
anyhow = "1.0"
thiserror = "1.0"
backoff = "0.4"  # Exponential backoff for retries
parking_lot = "0.12"  # Better synchronization primitives
crossbeam-channel = "0.5"  # Multi-producer multi-consumer channels
```

## Directory Structure

The current implementation has a simplified structure compared to the full plan above. Below is the **actual** directory structure:

```
synddb-sequencer/
├── Cargo.toml
├── src/
│   ├── main.rs                    # Entry point, CLI args, signing key management
│   ├── lib.rs                     # Public API and module exports
│   ├── config.rs                  # Configuration structures (clap + env vars)
│   ├── http_api.rs                # HTTP receiver (Axum endpoints for changesets/snapshots)
│   ├── http_errors.rs             # HTTP error handling
│   ├── inbox.rs                   # Message sequencing and ordering
│   ├── signer.rs                  # secp256k1 signing logic
│   ├── attestation.rs             # TEE attestation token fetching and verification
│   └── publish/
│       ├── mod.rs                 # Publishing orchestration and DAPublisher trait re-exports
│       ├── traits.rs              # DAPublisher trait definition
│       ├── gcs.rs                 # Google Cloud Storage publisher
│       ├── celestia.rs            # Celestia DA publisher
│       ├── eigenda.rs             # EigenDA publisher
│       ├── ipfs.rs                # IPFS publisher
│       ├── arweave.rs             # Arweave publisher
│       └── mock.rs                # Mock publisher for testing
└── README.md
```

**Note**: The planned directories (`batch/`, `attestor/`, `messages/`, `tee/`, `utils/`) have been consolidated:
- Batching logic is in `inbox.rs`
- Signing is in `signer.rs`
- Attestation is in `attestation.rs`
- Message passing details are described in `PLAN_MESSAGE_PASSING.md`

## Core Components

### 1. HTTP Receiver (Changesets + Snapshots from Client Libraries)

**Status**: ✅ **Implemented in `synddb-client` crate**

The original plan included a Session Monitor component that would directly monitor SQLite databases. This functionality has been implemented as the **SyndDB Client Library** (`crates/synddb-client/`) which applications embed directly.

**Client Library Implementation** (see `crates/synddb-client/src/`):
- `session.rs` - SessionMonitor using SQLite Session Extension
- `sender.rs` - ChangesetSender for batching and HTTP delivery
- `snapshot_sender.rs` - SnapshotSender for periodic and schema-triggered snapshots
- `attestation.rs` - GCP Confidential Space TEE attestation
- `recovery.rs` - Failed batch persistence for retry

**What the Sequencer Needs** (HTTP receiver - to be implemented):
- HTTP endpoint to receive changesets and snapshots from client libraries
- Verify TEE attestation tokens from clients
- Validate received data (checksums, sequence numbers)
- Queue operations for batching and DA publishing

### 2. Batcher

Accumulates received changesets and snapshots, then triggers publishing based on time/size thresholds.

**Note**: Snapshot creation is now handled by the client library. The sequencer just receives and batches them for DA publishing.

```rust
// src/batch/accumulator.rs
pub struct Batcher {
    db_path: PathBuf,
    changesets: Vec<Changeset>,
    buffer_size: usize,
    last_flush: Instant,
    last_snapshot: Instant,
    batch_count: usize,
    sequence: AtomicU64,
    config: BatchConfig,
}

pub struct BatchConfig {
    max_batch_size: usize,      // 1MB default
    max_batch_age: Duration,    // 1 second default
    snapshot_interval: Duration, // 60 mins default
    snapshot_threshold: usize,   // 1000 batches default
}

#[derive(Debug, Clone)]
pub enum BatchPayload {
    /// Full database snapshot triggered by schema change
    /// Includes DDL statements for audit trail
    SnapshotWithSchemaChange {
        snapshot_data: Vec<u8>,
        schema_change: SchemaChange,
        sequence: u64,
        timestamp: SystemTime,
    },
    /// Incremental changesets (deterministic replay)
    ChangesetBatch {
        changesets: Vec<Changeset>,
        sequence: u64,
        timestamp: SystemTime,
    },
    /// Full database snapshot (periodic recovery point)
    Snapshot {
        data: Vec<u8>,
        sequence: u64,
        timestamp: SystemTime,
    },
}

impl Batcher {
    pub async fn run(
        mut self,
        changeset_rx: Receiver<Changeset>,
        schema_rx: Receiver<SchemaChange>,
        tx: Sender<BatchPayload>
    ) {
        let mut flush_timer = tokio::time::interval(self.config.max_batch_age);

        loop {
            tokio::select! {
                Some(schema_change) = schema_rx.recv() => {
                    // Schema changes trigger immediate snapshot
                    // Discard pending changesets (snapshot includes all data)
                    self.changesets.clear();
                    self.buffer_size = 0;

                    // Create snapshot with current schema + data
                    let snapshot_data = self.create_snapshot().await?;

                    // Publish snapshot with schema change metadata
                    tx.send(BatchPayload::SnapshotWithSchemaChange {
                        snapshot_data,
                        schema_change: schema_change.clone(),
                        sequence: self.next_sequence(),
                        timestamp: SystemTime::now(),
                    }).await?;

                    warn!(
                        "SCHEMA CHANGE v{} -> v{}: Published snapshot ({} bytes)",
                        schema_change.old_version,
                        schema_change.new_version,
                        snapshot_data.len()
                    );

                    // Reset state (new epoch begins)
                    self.batch_count = 0;
                    self.last_snapshot = Instant::now();
                }
                Some(changeset) = changeset_rx.recv() => {
                    self.buffer_size += changeset.data.len();
                    self.changesets.push(changeset);

                    if self.should_flush() {
                        self.flush(&tx).await?;
                    }
                }
                _ = flush_timer.tick() => {
                    if !self.changesets.is_empty() {
                        self.flush(&tx).await?;
                    }
                }
            }
        }
    }

    fn should_flush(&self) -> bool {
        self.buffer_size >= self.config.max_batch_size ||
        self.last_flush.elapsed() >= self.config.max_batch_age
    }

    fn should_snapshot(&self) -> bool {
        self.last_snapshot.elapsed() >= self.config.snapshot_interval ||
        self.batch_count >= self.config.snapshot_threshold
    }

    async fn flush(&mut self, tx: &Sender<BatchPayload>) -> Result<()> {
        // Decide whether to send changesets or create snapshot
        if self.should_snapshot() {
            // Create full snapshot (recovery point for validators)
            let snapshot = self.create_snapshot().await?;
            tx.send(BatchPayload::Snapshot {
                data: snapshot,
                sequence: self.next_sequence(),
                timestamp: SystemTime::now(),
            }).await?;

            self.last_snapshot = Instant::now();
            self.batch_count = 0;
        } else {
            // Send incremental changesets (deterministic)
            let payload = BatchPayload::ChangesetBatch {
                changesets: self.changesets.drain(..).collect(),
                sequence: self.next_sequence(),
                timestamp: SystemTime::now(),
            };
            tx.send(payload).await?;
            self.batch_count += 1;
        }

        self.buffer_size = 0;
        self.last_flush = Instant::now();
        Ok(())
    }

    async fn create_snapshot(&self) -> Result<Vec<u8>> {
        // Use rusqlite backup API for consistent snapshot
        let conn = Connection::open(&self.db_path)?;
        let snapshot_path = format!("/tmp/snapshot-{}.db", self.sequence.load(Ordering::SeqCst));

        // Create backup
        let mut backup = conn.backup(
            rusqlite::DatabaseName::Main,
            Path::new(&snapshot_path),
            None
        )?;
        backup.step(-1)?; // Copy entire database

        // Read snapshot file
        let snapshot_data = tokio::fs::read(&snapshot_path).await?;
        tokio::fs::remove_file(&snapshot_path).await?;

        info!("Created database snapshot: {} bytes", snapshot_data.len());
        Ok(snapshot_data)
    }

    fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::SeqCst)
    }
}
```

### 3. Attestor (Compression + Signing)

The Attestor service compresses batches and signs them with TEE-protected Ethereum keys.
Following cryptographic best practices, we **compress-then-sign** to prevent signature malleability
and ensure validators verify authentic compressed data.

```rust
// src/attestor/mod.rs
use k256::ecdsa::{SigningKey, Signature, signature::Signer};
use zstd;

pub struct Attestor {
    key_manager: KeyManager,
    compression_level: i32,  // Zstd level 3 (default, auto-detected by validators)
}

#[derive(Serialize, Deserialize)]
pub struct AttestedPayload {
    pub payload_type: PayloadType,
    pub compressed_data: Vec<u8>,
    pub signature: Vec<u8>,
    pub public_key: Vec<u8>,
    pub sequence: u64,
    pub checksum: Blake3Hash,
    pub attestation_token: Option<String>,  // Fresh GCP attestation token
}

#[derive(Serialize, Deserialize)]
pub enum PayloadType {
    SnapshotWithSchemaChange,
    ChangesetBatch,
    Snapshot,
}

impl Attestor {
    pub async fn run(self, rx: Receiver<BatchPayload>, tx: Sender<AttestedPayload>) {
        while let Some(payload) = rx.recv().await {
            let attested = match payload {
                BatchPayload::SnapshotWithSchemaChange { snapshot_data, schema_change, sequence, .. } => {
                    self.attest_snapshot_with_schema_change(snapshot_data, schema_change, sequence).await?
                }
                BatchPayload::ChangesetBatch { changesets, sequence, .. } => {
                    self.attest_changeset_batch(changesets, sequence).await?
                }
                BatchPayload::Snapshot { data, sequence, .. } => {
                    self.attest_snapshot(data, sequence).await?
                }
            };

            tx.send(attested).await?;
        }
    }

    async fn attest_snapshot_with_schema_change(
        &self,
        snapshot_data: Vec<u8>,
        schema_change: SchemaChange,
        sequence: u64
    ) -> Result<AttestedPayload> {
        // Combine snapshot + schema metadata
        #[derive(Serialize)]
        struct SnapshotWithSchema {
            snapshot: Vec<u8>,
            schema_change: SchemaChange,
        }

        let combined = SnapshotWithSchema {
            snapshot: snapshot_data.clone(),
            schema_change: schema_change.clone(),
        };

        let serialized = bincode::serialize(&combined)?;
        let compressed = zstd::encode_all(&serialized[..], self.compression_level)?;
        let checksum = blake3::hash(&compressed);
        let signature = self.key_manager.sign(&compressed)?;
        let attestation_token = self.key_manager.get_attestation_token().await?;

        warn!(
            "Attested SNAPSHOT WITH SCHEMA CHANGE v{} -> v{}: {} bytes snapshot, {} DDL statements",
            schema_change.old_version,
            schema_change.new_version,
            snapshot_data.len(),
            schema_change.ddl_statements.len()
        );

        Ok(AttestedPayload {
            payload_type: PayloadType::SnapshotWithSchemaChange,
            compressed_data: compressed,
            signature: signature.to_vec(),
            public_key: self.key_manager.public_key_bytes(),
            sequence,
            checksum,
            attestation_token: Some(attestation_token),
        })
    }

    async fn attest_changeset_batch(&self, changesets: Vec<Changeset>, sequence: u64) -> Result<AttestedPayload> {
        // 1. Serialize changesets
        let serialized = bincode::serialize(&changesets)?;

        // 2. Compress with Zstd level 3 (good balance of speed/ratio)
        //    Validators auto-detect this via magic bytes (0x28 0xB5 0x2F 0xFD)
        let compressed = zstd::encode_all(&serialized[..], self.compression_level)?;
        let checksum = blake3::hash(&compressed);

        // 3. Sign compressed data (compress-then-sign prevents malleability)
        let signature = self.key_manager.sign(&compressed)?;

        // 4. Get fresh attestation token from GCP metadata service
        let attestation_token = self.key_manager.get_attestation_token().await?;

        info!(
            "Attested changeset batch: {} changesets, {} ops, {} bytes compressed",
            changesets.len(),
            changesets.iter().map(|c| c.operation_count).sum::<usize>(),
            compressed.len()
        );

        Ok(AttestedPayload {
            payload_type: PayloadType::ChangesetBatch,
            compressed_data: compressed,
            signature: signature.to_vec(),
            public_key: self.key_manager.public_key_bytes(),
            sequence,
            checksum,
            attestation_token: Some(attestation_token),
        })
    }

    async fn attest_snapshot(&self, data: Vec<u8>, sequence: u64) -> Result<AttestedPayload> {
        // Snapshots are raw database bytes
        let compressed = zstd::encode_all(&data[..], self.compression_level)?;
        let checksum = blake3::hash(&compressed);
        let signature = self.key_manager.sign(&compressed)?;
        let attestation_token = self.key_manager.get_attestation_token().await?;

        info!(
            "Attested snapshot: {} bytes raw, {} bytes compressed ({:.1}% ratio)",
            data.len(),
            compressed.len(),
            (compressed.len() as f64 / data.len() as f64) * 100.0
        );

        Ok(AttestedPayload {
            payload_type: PayloadType::Snapshot,
            compressed_data: compressed,
            signature: signature.to_vec(),
            public_key: self.key_manager.public_key_bytes(),
            sequence,
            checksum,
            attestation_token: Some(attestation_token),
        })
    }
}
```

### 4. Multi-DA Publisher

Publishes attested payloads to multiple DA layers in parallel with retry logic.
The Publisher waits for successful DA publication before acknowledging, ensuring reliable delivery.

```rust
// src/publish/mod.rs
pub struct Publisher {
    publishers: Vec<Box<dyn DaPublisher>>,
    retry_policy: RetryPolicy,
    manifest_store: ManifestStore,
}

#[async_trait]
pub trait DaPublisher: Send + Sync {
    async fn publish(&self, payload: &AttestedPayload) -> Result<PublishReceipt>;
    fn name(&self) -> &str;
}

impl Publisher {
    pub async fn run(self, rx: Receiver<AttestedPayload>) {
        while let Some(payload) = rx.recv().await {
            // Block until successfully published to all configured DA layers
            let results = self.publish_all(payload).await;

            // Record manifest (sequence -> DA locations mapping)
            self.manifest_store.record(results).await?;

            // Only after successful publication do we process next batch
            // This ensures reliable delivery and prevents data loss
        }
    }

    async fn publish_all(&self, payload: AttestedPayload) -> Vec<PublishResult> {
        // Publish to all DA layers in parallel
        let futures = self.publishers.iter().map(|publisher| {
            self.publish_with_retry(publisher, &payload)
        });

        futures::future::join_all(futures).await
    }

    async fn publish_with_retry(&self, publisher: &dyn DaPublisher, payload: &AttestedPayload) -> PublishResult {
        let backoff = ExponentialBackoff {
            max_elapsed_time: Some(Duration::from_secs(300)),  // 5 min max retry
            ..Default::default()
        };

        backoff::future::retry(backoff, || async {
            publisher.publish(payload).await
                .map_err(|e| {
                    if e.is_transient() {
                        backoff::Error::Transient(e)
                    } else {
                        backoff::Error::Permanent(e)
                    }
                })
        }).await
    }
}
```

### 5. DA Layer Implementations

Each DA layer has its own implementation:

```rust
// src/publish/celestia.rs
pub struct CelestiaPublisher {
    client: CelestiaClient,
    namespace: Namespace,
    gas_price: U256,
}

#[async_trait]
impl DaPublisher for CelestiaPublisher {
    async fn publish(&self, payload: &PublishPayload) -> Result<PublishReceipt> {
        let blob = match payload {
            PublishPayload::Diff(diff) => {
                Blob::new(self.namespace, diff.data.clone())?
            }
            PublishPayload::Snapshot(snapshot) => {
                // Split large snapshots into multiple blobs
                let chunks = snapshot.data.chunks(MAX_BLOB_SIZE);
                let blobs: Vec<Blob> = chunks.map(|chunk| {
                    Blob::new(self.namespace, chunk.to_vec())
                }).collect();

                // Publish all chunks
                for blob in blobs {
                    self.client.submit_blob(blob, self.gas_price).await?;
                }
            }
        };

        let tx_hash = self.client.submit_blob(blob, self.gas_price).await?;

        Ok(PublishReceipt {
            da_layer: "celestia",
            tx_hash: tx_hash.to_string(),
            timestamp: SystemTime::now(),
            sequence: payload.sequence(),
        })
    }
}

// src/publish/ipfs.rs
pub struct IpfsPublisher {
    client: IpfsClient,
    pin_remote: bool,
    pin_service: Option<PinataClient>,
}

#[async_trait]
impl DaPublisher for IpfsPublisher {
    async fn publish(&self, payload: &PublishPayload) -> Result<PublishReceipt> {
        let data = payload.as_bytes();

        // Add to local IPFS
        let cid = self.client.add(data).await?;

        // Pin to remote service for persistence
        if self.pin_remote {
            if let Some(pin_service) = &self.pin_service {
                pin_service.pin(cid.clone()).await?;
            }
        }

        Ok(PublishReceipt {
            da_layer: "ipfs",
            tx_hash: cid.to_string(),
            timestamp: SystemTime::now(),
            sequence: payload.sequence(),
        })
    }
}
```


## Message Passing - Bidirectional Bridge Communication

Message passing implementation has been extracted to a separate document for detailed coverage.

See **PLAN_MESSAGE_PASSING.md** for:
- Bidirectional message flow (blockchain ↔ application)
- Single writer model and read-only sequencer monitoring
- HTTP API specifications for inbound message delivery
- Message table monitoring strategies
- Consistency enforcement and progressive degradation
- State commitments and validator verification
- Edge cases and recovery protocols

## Configuration

```yaml
# config/default.yaml
database:
  path: "/data/app.db"
  wal_mode: true

monitor:
  poll_interval_ms: 100
  page_cache_size: 1000

batch:
  max_batch_size: 1048576 # 1MB
  max_batch_age_ms: 1000 # 1 second
  optimize_sql: true

compression:
  strategy: "zstd"
  zstd_level: 3
  snapshot_interval_mins: 60
  snapshot_threshold_diffs: 1000

publishers:
  celestia:
    enabled: true
    endpoint: "https://rpc.celestia.org"
    namespace: "0x00000000000000000000000000000000synddb"
    gas_price: "0.01"
    retry_attempts: 3

  eigenda:
    enabled: false
    endpoint: "https://disperser.eigenda.xyz"
    quorum_threshold: 67

  ipfs:
    enabled: true
    endpoint: "http://localhost:5001"
    pin_remote: true
    pinata_api_key: "${PINATA_API_KEY}"

  arweave:
    enabled: false
    endpoint: "https://arweave.net"
    wallet_path: "/secrets/arweave-wallet.json"

tee:
  enabled: false
  attestation_type: "dcap" # or "epid"
  mrenclave: "0x..."
  mrsigner: "0x..."

metrics:
  enabled: true
  port: 9090

logging:
  level: "info"
  format: "json"
```

## TEE Integration with GCP Confidential Space

The sequencer leverages GCP Confidential Space for hardware-protected Ethereum key management.
The sequencer runs in a **separate container** from the application within the same Confidential Space VM,
providing strong isolation while maintaining filesystem access to the SQLite database.

### Security Model: Same-VM, Separate Containers

```
┌──────────────────────────────────────────────────────────────────┐
│              GCP Confidential Space VM (TEE)                     │
│  Hardware Root of Trust (AMD SEV-SNP / Intel TDX)               │
│                                                                   │
│  ┌────────────────────────┐  ┌────────────────────────┐        │
│  │   Application          │  │   synddb-sequencer       │        │
│  │   Container            │  │   Container            │        │
│  │                        │  │                        │        │
│  │  - Any language        │  │  - Read-only SQLite    │        │
│  │  - Writes to SQLite    │  │  - Ethereum keys       │        │
│  │  - NO key access       │  │  - Signs batches       │        │
│  │                        │  │  - Publishes to DA     │        │
│  └────────────────────────┘  └────────────────────────┘        │
│             │                            │                       │
│             └────────┬───────────────────┘                       │
│                      ↓                                           │
│           ┌──────────────────────┐                              │
│           │  Shared Persistent   │                              │
│           │  Disk (SQLite DB)    │                              │
│           └──────────────────────┘                              │
│                                                                   │
│  Container-level isolation prevents application from accessing   │
│  sequencer memory space where Ethereum keys are held.           │
└──────────────────────────────────────────────────────────────────┘
                     ↓ Attestation Token + Signature
┌──────────────────────────────────────────────────────────────────┐
│                        Bridge.sol                                │
│  - Verify attestation via SP1 zkVM                              │
│  - Register public key after verification                       │
│  - Track valid signers via ecrecover                            │
└──────────────────────────────────────────────────────────────────┘
```

### Why Same-VM Architecture is Secure

1. **Container Isolation**: Linux namespaces and cgroups prevent cross-container memory access
2. **Read-Only SQLite Access**: Sequencer opens DB with `SQLITE_OPEN_READ_ONLY` flag
3. **Memory Encryption**: AMD SEV-SNP encrypts all VM memory including both containers
4. **No Shared Memory**: Containers communicate only via filesystem (SQLite DB file)
5. **Principle of Least Privilege**: Application has no credentials to access Secret Manager
6. **Attestation Binding**: Keys in Secret Manager are bound to sequencer container digest only

### Key Management Implementation

```rust
// src/attestor/key_manager.rs
use gcp_auth::AuthenticationManager;
use google_cloud_secretmanager::client::{Client as SecretClient, ClientConfig};
use google_cloud_default::WithAuthExt;
use serde::{Deserialize, Serialize};
use k256::ecdsa::{SigningKey, VerifyingKey, Signature, signature::Signer};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use sha2::{Sha256, Digest};
use anyhow::Result;

/// Manages Ethereum secp256k1 keys for signing batches.
/// Keys are generated in TEE and stored in GCP Secret Manager,
/// bound to the sequencer container digest via Workload Identity.
pub struct KeyManager {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
    ethereum_address: [u8; 20],
    secret_client: SecretClient,
    project_id: String,
    secret_name: String,
}

#[derive(Serialize, Deserialize)]
struct SealedKeyData {
    /// secp256k1 private key (32 bytes)
    private_key: Vec<u8>,
    /// secp256k1 public key (compressed, 33 bytes)
    public_key: Vec<u8>,
    /// Ethereum address derived from public key
    ethereum_address: [u8; 20],
    created_at: i64,
    /// Attestation token at key creation
    attestation_token: String,
}

impl KeyManager {
    pub async fn init() -> Result<Self> {
        // Get project ID from metadata service
        let project_id = Self::get_project_id().await?;

        // Initialize Secret Manager client with Workload Identity
        let config = ClientConfig::default().with_auth().await?;
        let secret_client = SecretClient::new(config).await?;

        let secret_name = "synddb-sequencer-signing-key".to_string();

        // Try to load existing key or generate new one
        let (signing_key, verifying_key, ethereum_address) =
            match Self::load_sealed_key(&secret_client, &project_id, &secret_name).await {
                Ok(key_data) => {
                    info!("Loaded existing Ethereum signing key from Secret Manager");
                    let signing_key = SigningKey::from_slice(&key_data.private_key)?;
                    let verifying_key = VerifyingKey::from_sec1_bytes(&key_data.public_key)?;
                    (signing_key, verifying_key, key_data.ethereum_address)
                }
                Err(_) => {
                    info!("Generating new Ethereum signing key (secp256k1)");

                    // Generate secp256k1 key pair
                    let signing_key = SigningKey::random(&mut rand::thread_rng());
                    let verifying_key = signing_key.verifying_key();

                    // Derive Ethereum address from public key
                    let ethereum_address = Self::derive_ethereum_address(&verifying_key);

                    // Get attestation token for this container
                    let attestation_token = Self::get_attestation_token().await?;

                    // Seal to Secret Manager (bound to container digest via WI)
                    Self::seal_key(
                        &secret_client,
                        &project_id,
                        &secret_name,
                        &signing_key,
                        &verifying_key,
                        ethereum_address,
                        &attestation_token
                    ).await?;

                    (signing_key, verifying_key, ethereum_address)
                }
            };

        Ok(Self {
            signing_key,
            verifying_key,
            ethereum_address,
            secret_client,
            project_id,
            secret_name,
        })
    }

    /// Derive Ethereum address from secp256k1 public key
    /// Address = keccak256(pubkey)[12:32]
    fn derive_ethereum_address(verifying_key: &VerifyingKey) -> [u8; 20] {
        use sha3::{Keccak256, Digest};

        // Get uncompressed public key (65 bytes: 0x04 + x + y)
        let public_key_bytes = verifying_key.to_encoded_point(false);
        let public_key_slice = &public_key_bytes.as_bytes()[1..]; // Skip 0x04 prefix

        // Hash with Keccak256
        let mut hasher = Keccak256::new();
        hasher.update(public_key_slice);
        let hash = hasher.finalize();

        // Take last 20 bytes
        let mut address = [0u8; 20];
        address.copy_from_slice(&hash[12..32]);
        address
    }

    async fn get_attestation_token() -> Result<String> {
        // Get attestation token from Confidential Space metadata service
        let client = reqwest::Client::new();

        // Custom audience for our application
        let audience = "https://synddb.io/sequencer";

        let response = client
            .get("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token")
            .query(&[
                ("audience", audience),
                ("format", "full"),
                ("licenses", "TRUE"),  // Include container image measurements
            ])
            .header("Metadata-Flavor", "Google")
            .send()
            .await?;

        #[derive(Deserialize)]
        struct TokenResponse {
            token: String,
        }

        let token_resp: TokenResponse = response.json().await?;
        Ok(token_resp.token)
    }

    async fn seal_key(
        secret_client: &SecretClient,
        project_id: &str,
        secret_name: &str,
        signing_key: &SigningKey,
        verifying_key: &VerifyingKey,
        ethereum_address: [u8; 20],
        attestation_token: &str,
    ) -> Result<()> {
        let key_data = SealedKeyData {
            private_key: signing_key.to_bytes().to_vec(),
            public_key: verifying_key.to_encoded_point(true).as_bytes().to_vec(), // Compressed
            ethereum_address,
            created_at: chrono::Utc::now().timestamp(),
            attestation_token: attestation_token.to_string(),
        };

        let secret_data = serde_json::to_vec(&key_data)?;

        // Create secret with Workload Identity binding
        // IAM policy ensures only sequencer container with matching digest can access
        secret_client
            .create_secret(
                project_id,
                secret_name,
                secret_data,
                Some(vec![
                    ("synddb/environment", "confidential-space"),
                    ("synddb/component", "sequencer"),
                    ("synddb/key-type", "secp256k1-ethereum"),
                ]),
            )
            .await?;

        info!("Sealed Ethereum key to Secret Manager: 0x{}", hex::encode(ethereum_address));
        Ok(())
    }

    async fn load_sealed_key(
        secret_client: &SecretClient,
        project_id: &str,
        secret_name: &str,
    ) -> Result<SealedKeyData> {
        let secret_data = secret_client
            .access_secret_version(project_id, secret_name, "latest")
            .await?;

        let key_data: SealedKeyData = serde_json::from_slice(&secret_data)?;
        Ok(key_data)
    }

    /// Sign data with secp256k1 key (returns recoverable signature)
    pub fn sign(&self, data: &[u8]) -> Result<Signature> {
        use k256::ecdsa::signature::Signer;
        Ok(self.signing_key.sign(data))
    }

    /// Get public key bytes (compressed, 33 bytes)
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.verifying_key.to_encoded_point(true).as_bytes().to_vec()
    }

    /// Get Ethereum address
    pub fn ethereum_address(&self) -> [u8; 20] {
        self.ethereum_address
    }

    pub async fn get_attestation_token(&self) -> Result<String> {
        Self::get_attestation_token().await
    }

    async fn get_project_id() -> Result<String> {
        let client = reqwest::Client::new();
        let response = client
            .get("http://metadata.google.internal/computeMetadata/v1/project/project-id")
            .header("Metadata-Flavor", "Google")
            .send()
            .await?;
        Ok(response.text().await?)
    }
}
```

### Docker Configuration

```dockerfile
# Dockerfile.confidential
FROM rust:1.75 as builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build with Confidential Space features
RUN cargo build --release --features confidential-space

# Runtime image for Confidential Space
FROM gcr.io/confidential-space-images/base:latest

# Install required dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy binary
COPY --from=builder /app/target/release/synddb-sequencer /usr/local/bin/

# Create non-root user
RUN useradd -m -u 1000 synddb && \
    chown -R synddb:synddb /usr/local/bin/synddb-sequencer

USER synddb

# Health check
HEALTHCHECK --interval=30s --timeout=3s \
    CMD curl -f http://localhost:9090/health || exit 1

# Entry point with attestation initialization
ENTRYPOINT ["/usr/local/bin/synddb-sequencer"]
CMD ["--attestation", "confidential-space", "--config", "/config/sequencer.yaml"]
```

### Deployment Configuration

```yaml
# confidential-space-deployment.yaml
apiVersion: compute.cnrm.cloud.google.com/v1beta1
kind: ComputeInstance
metadata:
  name: synddb-sequencer-tee
spec:
  machineType: n2d-standard-4 # AMD SEV-SNP capable
  zone: us-central1-a

  confidentialInstanceConfig:
    enableConfidentialCompute: true
    confidentialComputeType: SEV_SNP # or TDX for Intel

  shieldedInstanceConfig:
    enableSecureBoot: true
    enableVtpm: true
    enableIntegrityMonitoring: true

  scheduling:
    onHostMaintenance: TERMINATE # Required for Confidential VMs

  serviceAccounts:
    - email: synddb-sequencer@${PROJECT_ID}.iam.gserviceaccount.com
      scopes:
        - https://www.googleapis.com/auth/cloud-platform

  metadata:
    items:
      - key: tee-container-log-redirect
        value: "true"
      - key: tee-image-reference
        value: "gcr.io/${PROJECT_ID}/synddb-sequencer:latest"
      - key: tee-restart-policy
        value: "Always"
      - key: tee-env-ATTESTATION_AUDIENCE
        value: "https://synddb.io/sequencer"
      - key: tee-env-SECRET_PROJECT_ID
        value: "${PROJECT_ID}"

  bootDisk:
    initializeParams:
      image: projects/confidential-space-images/global/images/confidential-space-release
```

### Workload Identity Configuration

```yaml
# workload-identity.yaml
apiVersion: iam.cnrm.cloud.google.com/v1beta1
kind: IAMServiceAccount
metadata:
  name: synddb-sequencer
spec:
  displayName: SyndDB Sequencer Service Account
---
apiVersion: iam.cnrm.cloud.google.com/v1beta1
kind: IAMPolicyMember
metadata:
  name: synddb-sequencer-secretmanager
spec:
  memberFrom:
    serviceAccountRef:
      name: synddb-sequencer
  role: roles/secretmanager.secretAccessor
  resourceRef:
    kind: Project
---
apiVersion: iam.cnrm.cloud.google.com/v1beta1
kind: IAMPolicy
metadata:
  name: synddb-sequencer-secret-policy
spec:
  resourceRef:
    kind: Secret
    name: synddb-sequencer-signing-key
  policy:
    bindings:
      - role: roles/secretmanager.secretAccessor
        members:
          - serviceAccount:synddb-sequencer@${PROJECT_ID}.iam.gserviceaccount.com
        condition:
          title: Only from Confidential Space
          expression: |
            assertion.sub == 'synddb-sequencer@${PROJECT_ID}.iam.gserviceaccount.com' &&
            'image_digest' in assertion &&
            assertion.image_digest == '${EXPECTED_IMAGE_DIGEST}'
```

### Configuration File

```yaml
# config/sequencer-confidential.yaml
database:
  path: "/data/app.db"
  wal_mode: true

attestation:
  enabled: true
  provider: "gcp-confidential-space"

  # GCP Confidential Space settings
  gcp:
    project_id: "${PROJECT_ID}"
    secret_name: "synddb-sequencer-signing-key"
    attestation_audience: "https://synddb.io/sequencer"

    # Expected measurements for verification
    expected_measurements:
      container_image_digest: "${EXPECTED_IMAGE_DIGEST}"

  # How often to refresh attestation
  refresh_interval_mins: 60

  # Bridge contract for key registration
  bridge_contract: "0x..."
  bridge_rpc: "https://eth-mainnet.g.alchemy.com/v2/..."

publishers:
  celestia:
    enabled: true
    endpoint: "https://rpc.celestia.org"
    # Include attestation with all published data
    include_attestation: true

monitoring:
  metrics:
    enabled: true
    port: 9090
  health:
    enabled: true
    port: 8080
```

## Additional Operational Considerations

### 1. Disaster Recovery & Persistence ⚠️ TODO

**Problem:** If sequencer crashes, in-memory changesets are lost
**Solution:** Persistent queue before DA publish
```rust
// src/publish/persistent_queue.rs
pub struct PersistentQueue {
    disk_buffer: sled::Db,  // Fast embedded key-value store
    sequence_tracker: AtomicU64,
}

impl PersistentQueue {
    // Write changeset to disk before publishing
    pub fn enqueue(&self, payload: AttestedPayload) -> Result<()> {
        self.disk_buffer.insert(
            &payload.sequence.to_be_bytes(),
            bincode::serialize(&payload)?
        )?;
        Ok(())
    }

    // Mark as published after DA confirmation
    pub fn dequeue(&self, sequence: u64) -> Result<()> {
        self.disk_buffer.remove(&sequence.to_be_bytes())?;
        Ok(())
    }

    // On restart, republish any unconfirmed payloads
    pub fn recover(&self) -> Result<Vec<AttestedPayload>> {
        let mut pending = Vec::new();
        for item in self.disk_buffer.iter() {
            let (_, payload_bytes) = item?;
            pending.push(bincode::deserialize(&payload_bytes)?);
        }
        Ok(pending)
    }
}
```

**Status:** ⚠️ TODO - Add persistent queue between Attestor and Publisher

### 2. Backpressure & Memory Management ⚠️ TODO

**Problem:** Application writes faster than sequencer can publish
**Solution:** Bounded channels with monitoring
```rust
// Bounded channels prevent memory exhaustion
let (changeset_tx, changeset_rx) = mpsc::channel::<Changeset>(1000);
let (batch_tx, batch_rx) = mpsc::channel::<BatchPayload>(100);

// Monitor queue depth
if changeset_tx.capacity() < 100 {
    warn!("Sequencer falling behind: {} changesets queued",
          1000 - changeset_tx.capacity());
}
```

**Status:** ⚠️ TODO - Add queue depth monitoring and alerts

### 3. Graceful Shutdown ⚠️ TODO

**Problem:** Sequencer shutdown could lose in-flight changesets
**Solution:** Flush all pending work before exit
```rust
impl Sequencer {
    pub async fn shutdown(&mut self) -> Result<()> {
        info!("Graceful shutdown initiated");

        // 1. Stop accepting new changesets
        self.session_monitor.stop().await?;

        // 2. Flush all pending batches
        self.batcher.flush_all(&self.attestor_tx).await?;

        // 3. Wait for attestor to finish
        self.attestor.wait_idle().await?;

        // 4. Wait for all DA publishes to confirm
        self.publisher.wait_for_pending().await?;

        // 5. Persist state checkpoint
        self.save_checkpoint().await?;

        info!("Graceful shutdown complete");
        Ok(())
    }

    async fn save_checkpoint(&self) -> Result<()> {
        let state = SequencerState {
            last_published_sequence: self.sequence.load(Ordering::SeqCst),
            last_schema_version: self.schema_version,
        };
        fs::write("/data/sequencer.checkpoint", serde_json::to_vec(&state)?)?;
        Ok(())
    }
}
```

**Status:** ⚠️ TODO - Implement graceful shutdown

### 4. Large Transaction Handling ⚠️ TODO

**Problem:** Single transaction with millions of rows creates huge changeset
**Solution:** Detect large changesets and force snapshot instead
```rust
impl Batcher {
    const MAX_CHANGESET_SIZE: usize = 100_000_000; // 100MB

    async fn flush(&mut self, tx: &Sender<BatchPayload>) -> Result<()> {
        let batch_size: usize = self.changesets.iter()
            .map(|c| c.data.len())
            .sum();

        if batch_size > Self::MAX_CHANGESET_SIZE {
            warn!("Large transaction detected ({} bytes), forcing snapshot", batch_size);

            // Discard changesets, create snapshot instead
            self.changesets.clear();
            let snapshot_data = self.create_snapshot().await?;

            tx.send(BatchPayload::Snapshot {
                data: snapshot_data,
                sequence: self.next_sequence(),
                timestamp: SystemTime::now(),
            }).await?;
        } else {
            // Normal changeset batch
            // ...
        }
    }
}
```

**Status:** ⚠️ TODO - Add large transaction detection

### 5. Key Rotation ⚠️ TODO

**Problem:** Ethereum signing keys may need rotation for security
**Solution:** Publish key rotation messages to validators
```rust
pub struct KeyRotation {
    old_public_key: Vec<u8>,
    new_public_key: Vec<u8>,
    rotation_sequence: u64,
    attestation_token: String,  // Proves new key is from same TEE
}

impl KeyManager {
    pub async fn rotate_key(&mut self) -> Result<KeyRotation> {
        let old_key = self.verifying_key.clone();

        // Generate new key
        let new_signing_key = SigningKey::random(&mut rand::thread_rng());
        let new_verifying_key = new_signing_key.verifying_key();

        // Get attestation for new key
        let attestation = self.get_attestation_token().await?;

        // Seal new key to Secret Manager
        self.seal_key(&new_signing_key, &new_verifying_key, &attestation).await?;

        // Keep both keys active during transition
        self.old_signing_key = Some(self.signing_key);
        self.signing_key = new_signing_key;

        Ok(KeyRotation {
            old_public_key: old_key.to_encoded_point(true).as_bytes().to_vec(),
            new_public_key: new_verifying_key.to_encoded_point(true).as_bytes().to_vec(),
            rotation_sequence: self.current_sequence(),
            attestation_token: attestation,
        })
    }
}
```

**Status:** ⚠️ TODO - Implement key rotation protocol

### 6. Observability & Monitoring ⚠️ TODO

**Critical Metrics:**
```rust
// GCP Cloud Logging structured events
pub struct SequencerMetrics {
    // Lag metrics
    pub changeset_lag_seconds: f64,        // Time from commit to publish
    pub queue_depth: usize,                 // Unpublished changesets

    // Throughput metrics
    pub changesets_per_second: f64,
    pub bytes_published_per_second: f64,

    // Error metrics
    pub da_publish_failures: u64,
    pub schema_detection_errors: u64,

    // Health metrics
    pub session_extension_healthy: bool,
    pub db_connection_healthy: bool,
    pub da_layers_reachable: HashMap<String, bool>,
}
```

**Status:** ⚠️ TODO - Add comprehensive metrics

### 7. Supported SQLite Features

**Known Limitations:**
- ❌ Memory databases (`:memory:`) - Cannot persist
- ❌ Temporary tables - Not replicated by Session Extension
- ⚠️ ATTACH DATABASE - Each database needs separate sequencer
- ⚠️ Encrypted databases (SQLCipher) - Need special handling
- ⚠️ Custom collations - Must match on validators

**Status:** ⚠️ TODO - Document and test feature compatibility

## Performance Optimizations

### 1. Zero-Copy WAL Reading

Read WAL frames directly without copying:

```rust
use memmap2::MmapOptions;

let file = File::open(&wal_path)?;
let mmap = unsafe { MmapOptions::new().map(&file)? };
// Parse WAL frames directly from memory-mapped file
```

### 2. Parallel Compression

Use all CPU cores for compression:

```rust
use rayon::prelude::*;

let compressed_chunks: Vec<_> = chunks
    .par_iter()
    .map(|chunk| zstd::encode_all(chunk, level))
    .collect();
```

### 3. Batched Publishing

Combine multiple small payloads:

```rust
let mut batch = Vec::new();
let mut batch_size = 0;

while let Ok(payload) = rx.try_recv() {
    batch.push(payload);
    batch_size += payload.size();

    if batch_size > MAX_BATCH_SIZE {
        break;
    }
}

publisher.publish_batch(batch).await?;
```

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_wal_parsing() {
        let wal_data = include_bytes!("../fixtures/sample.wal");
        let frames = parse_wal_frames(wal_data).unwrap();
        assert_eq!(frames.len(), 42);
    }

    #[tokio::test]
    async fn test_batch_trigger() {
        let batcher = Batcher::new(config);
        // Test time-based and size-based triggers
    }
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_end_to_end() {
    // Start test SQLite database
    let db = setup_test_db().await;

    // Start sequencer
    let sequencer = Sequencer::new(config).start().await;

    // Perform SQL operations
    db.execute("INSERT INTO test VALUES (1, 'data')").await;

    // Verify published to mock DA layers
    let published = mock_da.get_published().await;
    assert_eq!(published.len(), 1);
}
```

### Benchmarks

```rust
use criterion::{criterion_group, criterion_main, Criterion};

fn bench_compression(c: &mut Criterion) {
    c.bench_function("zstd_1mb", |b| {
        let data = vec![0u8; 1_000_000];
        b.iter(|| zstd::encode_all(&data[..], 3))
    });
}
```

## Deployment

### Docker Image

```dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release --features tee

FROM ubuntu:22.04
RUN apt-get update && apt-get install -y ca-certificates
COPY --from=builder /app/target/release/synddb-sequencer /usr/local/bin/
ENTRYPOINT ["synddb-sequencer"]
```

### Resource Requirements

- **CPU**: 2 cores minimum (4 recommended)
- **Memory**: 512MB minimum (2GB recommended)
- **Disk**: 10GB for caching
- **Network**: 100Mbps for DA publishing

### Monitoring

All metrics and logs are sent to GCP Cloud Logging for centralized monitoring:

**Structured Log Events:**
- `wal_frames_processed` - WAL frame count and processing latency
- `batch_published` - Batch sequence, size, DA layer, publish latency
- `da_publish_latency` - Per-DA layer publish times
- `compression_ratio` - Compression effectiveness metrics
- `attestation_refresh` - Attestation token refresh events
- `key_loaded` - Ethereum key loaded from Secret Manager

**Health Endpoint:**
- Port 8080: `/health` - Basic health check
- Port 8080: `/metrics` - JSON metrics endpoint for external monitoring

## Security Considerations

### 1. Read-Only Access

Sequencer only reads from SQLite, never writes:

```rust
let conn = Connection::open_with_flags(
    &db_path,
    OpenFlags::SQLITE_OPEN_READ_ONLY
)?;
```

### 2. Data Integrity

All published data includes checksums:

```rust
pub struct PublishPayload {
    data: Vec<u8>,
    checksum: Blake3Hash,
    sequence: u64,
    attestation: Option<TeeAttestation>,
}
```

### 3. TEE Isolation and Key Security

Ethereum signing keys are protected by multiple layers:

1. **Container Isolation**: Application container cannot access sequencer memory
2. **Secret Manager Binding**: Keys only accessible to container with matching digest
3. **Memory Encryption**: AMD SEV-SNP encrypts all VM memory
4. **No Key Export**: Keys never serialized outside Secret Manager

```rust
// Sequencer loads key from Secret Manager on startup
let key_manager = KeyManager::init().await?;

// Application has no access to Secret Manager or sequencer memory
// Keys remain in sequencer service memory only (separate TEE from application)
```

### 4. Rate Limiting

Prevent resource exhaustion:

```rust
use governor::{Quota, RateLimiter};

let limiter = RateLimiter::direct(Quota::per_second(100));
limiter.until_ready().await;
```

## Migration Path

For existing applications:

1. **Ensure WAL mode**:

```sql
PRAGMA journal_mode=WAL;
```

2. **Deploy sequencer**:

```bash
docker run -v /app/data:/data syndicate/synddb-sequencer
```

3. **Verify publishing**:

```bash
curl http://localhost:9090/metrics | grep synddb_
```

No application code changes required - the sequencer is completely passive and transparent to the application.
