# PLAN_SIDECAR.md - Lightweight SQLite Monitor and Publisher

## Overview

The synddb-sidecar is a zero-configuration Rust process that attaches to any SQLite database via WAL (Write-Ahead Logging) monitoring, captures all SQL operations, and publishes them to multiple DA layers. It requires zero application changes and works with SQLite databases from any programming language.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Application (Any Language)              │
│  Python/JS/Go/Rust/Java → SQLite API → app.db             │
└─────────────────────────────────────────────────────────────┘
                                │
                        WAL File Changes
                                ↓
┌─────────────────────────────────────────────────────────────┐
│                      synddb-sidecar                         │
│ ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│ │ WAL Monitor  │→ │   Batcher    │→ │  Compressor  │      │
│ └──────────────┘  └──────────────┘  └──────────────┘      │
│         ↓                                     ↓              │
│ ┌──────────────┐                    ┌──────────────┐      │
│ │ SQL Extractor│                    │  Snapshotter │      │
│ └──────────────┘                    └──────────────┘      │
│         ↓                                     ↓              │
│ ┌────────────────────────────────────────────────┐        │
│ │            Multi-DA Publisher                    │        │
│ │  ┌─────────┐ ┌─────────┐ ┌──────┐ ┌─────────┐ │        │
│ │  │Celestia │ │EigenDA  │ │ IPFS │ │ Arweave │ │        │
│ │  └─────────┘ └─────────┘ └──────┘ └─────────┘ │        │
│ └────────────────────────────────────────────────┘        │
└─────────────────────────────────────────────────────────────┘
```

## Core Libraries

```toml
[dependencies]
# Core SQLite monitoring
rusqlite = { version = "0.32", features = ["bundled", "backup", "hooks"] }
notify = "6.1"  # File system monitoring for WAL changes

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

# CLAUDE: Is this needed for GCP Confidential Space TEEs?
# TEE support (optional)
sgx-isa = { version = "0.4", optional = true }
dcap-ql = { version = "0.3", optional = true }
teaclave-attestation = { version = "0.5", optional = true }

# CLAUDE: Since this is running in GCP Confidential Space, can we use Cloud Logging?
# Configuration and monitoring
config = "0.14"  # YAML/TOML configuration
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
prometheus = "0.13"  # Metrics
sysinfo = "0.30"  # System monitoring

# Error handling and utilities
anyhow = "1.0"
thiserror = "1.0"
backoff = "0.4"  # Exponential backoff for retries
parking_lot = "0.12"  # Better synchronization primitives
crossbeam-channel = "0.5"  # Multi-producer multi-consumer channels
```

## Directory Structure

```
synddb-sidecar/
├── Cargo.toml
├── src/
│   ├── main.rs                    # Entry point, CLI args
│   ├── lib.rs                     # Public API
│   ├── config.rs                  # Configuration structures
│   ├── monitor/
│   │   ├── mod.rs                 # WAL monitoring module
│   │   ├── wal_reader.rs          # Direct WAL file parsing
│   │   ├── page_cache.rs          # Track modified pages
│   │   └── sql_extractor.rs       # Extract SQL from WAL frames
CLAUDE: What is the purpose of the optimizier? We want to reproduce the SQLite operations deterministically. Either cut this or elaborate on its purpose
│   ├── batch/
│   │   ├── mod.rs                 # Batching logic
│   │   ├── accumulator.rs         # Accumulate operations
│   │   ├── optimizer.rs           # Optimize SQL sequences
│   │   └── timer.rs               # Time/size based triggers
│   ├── compress/
│   │   ├── mod.rs                 # Compression module
│   │   ├── diff.rs                # Diff compression
│   │   ├── snapshot.rs            # Full snapshot creation
│   │   └── strategies.rs          # Zstd, LZ4, etc.
│   ├── publish/
│   │   ├── mod.rs                 # Publishing orchestration
│   │   ├── celestia.rs            # Celestia publisher
│   │   ├── eigenda.rs             # EigenDA publisher
│   │   ├── ipfs.rs                # IPFS publisher
│   │   ├── arweave.rs             # Arweave publisher
│   │   ├── retry.rs               # Retry logic
CLAUDE: What are these manifests? Elaborate on this purpose slightly
│   │   └── manifest.rs            # Publish manifest tracking
CLAUDE: How much of this is necessary in GCP Confidential Space?
CLAUDE: Don't we need key management for the sidecar to sign and submit to the DA layer? That seems to be missing
│   ├── tee/
│   │   ├── mod.rs                 # TEE integration
│   │   ├── attestation.rs         # Generate attestations
│   │   ├── sealing.rs             # Seal data to TEE
│   │   └── remote_attestation.rs  # Remote attestation
CLAUDE: Can we replace this with GCP Cloud Logging? Happy to keep it if it's still relevant though!
│   ├── metrics/
│   │   ├── mod.rs                 # Prometheus metrics
│   │   └── collectors.rs          # Custom collectors
│   └── utils/
│       ├── mod.rs
│       ├── checksum.rs            # Data integrity
│       └── sqlite_utils.rs        # SQLite helpers
├── config/
│   ├── default.yaml               # Default configuration
│   └── example.yaml               # Example with all options
├── tests/
│   ├── integration/               # Integration tests
│   └── benchmarks/                # Performance benchmarks
└── README.md
```

## Core Components

### 1. WAL Monitor

The WAL monitor watches the SQLite WAL file for changes and extracts SQL operations:

```rust
// src/monitor/wal_reader.rs
pub struct WalMonitor {
    db_path: PathBuf,
    wal_path: PathBuf,
    last_frame: u32,
    page_cache: PageCache,
    sql_extractor: SqlExtractor,
}

impl WalMonitor {
    pub async fn start(self, tx: Sender<SqlBatch>) -> Result<()> {
        // Use inotify/fsevents to watch WAL file
        let mut watcher = notify::recommended_watcher(move |event| {
            if let Ok(Event::Modify(_)) = event {
                self.process_new_frames().await?;
            }
        })?;

        watcher.watch(&self.wal_path, RecursiveMode::NonRecursive)?;

        // CLAUDE: Is this idiomatic or is a fallback in the case of watch failures more idiomatic? I'm open to either option
        // Also poll periodically as backup
        let poll_interval = Duration::from_millis(100);
        loop {
            tokio::time::sleep(poll_interval).await;
            self.process_new_frames(&tx).await?;
        }
    }

    async fn process_new_frames(&self, tx: &Sender<SqlBatch>) -> Result<()> {
        // Read new WAL frames since last_frame
        let frames = self.read_wal_frames(self.last_frame)?;

        // CLAUDE: Is there any built-in SQLite utility that lets me get a diff?
        // Extract SQL operations from page changes
        let sql_ops = self.sql_extractor.extract(&frames)?;

        // Send to batcher
        if !sql_ops.is_empty() {
            tx.send(SqlBatch::new(sql_ops)).await?;
            self.last_frame = frames.last().map(|f| f.frame_number).unwrap_or(self.last_frame);
        }

        Ok(())
    }
}
```

### 2. Batcher

Accumulates SQL operations and triggers publishing based on time/size thresholds:

```rust
// src/batch/accumulator.rs
pub struct Batcher {
    buffer: Vec<SqlOperation>,
    buffer_size: usize,
    last_flush: Instant,
    config: BatchConfig,
}

pub struct BatchConfig {
    max_batch_size: usize,      // 1MB default
    max_batch_age: Duration,    // 1 second default
    // CLAUDE: Will this break deterministic derivation?
    optimize_sql: bool,          // Combine similar ops
}

impl Batcher {
    pub async fn run(mut self, rx: Receiver<SqlBatch>, tx: Sender<CompressedBatch>) {
        let mut flush_timer = tokio::time::interval(self.config.max_batch_age);

        loop {
            tokio::select! {
                Some(batch) = rx.recv() => {
                    self.add_batch(batch);
                    if self.should_flush() {
                        self.flush(&tx).await?;
                    }
                }
                _ = flush_timer.tick() => {
                    if !self.buffer.is_empty() {
                        self.flush(&tx).await?;
                    }
                }
            }
        }
    }

    // CLAUDE: Is it idiomatic to flush baesed on size and time like this? Or should we wait for confirmation that we've posted to a DA layer? We want to ensure that batch posting is extremely reliable
    fn should_flush(&self) -> bool {
        self.buffer_size >= self.config.max_batch_size ||
        self.last_flush.elapsed() >= self.config.max_batch_age
    }

    async fn flush(&mut self, tx: &Sender<CompressedBatch>) -> Result<()> {
        if self.config.optimize_sql {
            self.buffer = optimize_sql_sequence(self.buffer.drain(..).collect());
        }

        let batch = SqlBatch {
            operations: self.buffer.drain(..).collect(),
            timestamp: SystemTime::now(),
            sequence: self.next_sequence(),
        };

        tx.send(batch).await?;
        self.buffer_size = 0;
        self.last_flush = Instant::now();
        Ok(())
    }
}
```

### 3. Compressor

Handles both incremental diffs and periodic snapshots:

```rust
// src/compress/mod.rs
pub struct Compressor {
    strategy: CompressionStrategy,
    snapshot_interval: Duration,
    last_snapshot: Instant,
    snapshot_threshold: usize,  // Force snapshot if diff chain too long
}

// CLAUDE: Let's not overcomplicate this. We should just choose compression defaults that are set for all instances of the application. That being said, whatever compression default we choose, we should ensure that it is easy to auto-detect for validators after downloading from the DA layer
// CLAUDE: We do need to handle signing somewhere, and the batcher feels like the wrong place. Maybe the Compressor service is dedicated to signing and compressing? That makes the Compressor more useful and also ensures that all messages are signed, either before or after compression (I know that cryptographers have strong opinions here, cite best practices on whether we should sign -> compress or compress -> sign)
// CLAUDE: If we merge signing and compression into the same service, we'll obviously need to rename it to something else. Give a proposal here
pub enum CompressionStrategy {
    Zstd { level: i32 },
    Lz4 { acceleration: i32 },
    None,
}

impl Compressor {
    pub async fn run(self, rx: Receiver<SqlBatch>, tx: Sender<PublishPayload>) {
        let mut diff_count = 0;

        while let Some(batch) = rx.recv().await {
            if self.should_snapshot(diff_count) {
                let snapshot = self.create_snapshot().await?;
                tx.send(PublishPayload::Snapshot(snapshot)).await?;
                diff_count = 0;
            } else {
                let diff = self.compress_diff(batch).await?;
                tx.send(PublishPayload::Diff(diff)).await?;
                diff_count += 1;
            }
        }
    }

    // CLAUDE: Does this data flow make sense, or should the batcher tell the Compressor when to trigger a snapshot? It seems like if timing logic is in the batcher already, we should just stick with that (but I'm open to other ideas)
    fn should_snapshot(&self, diff_count: usize) -> bool {
        self.last_snapshot.elapsed() >= self.snapshot_interval ||
        diff_count >= self.snapshot_threshold
    }

    async fn compress_diff(&self, batch: SqlBatch) -> Result<CompressedDiff> {
        let serialized = bincode::serialize(&batch)?;
        let compressed = match self.strategy {
            CompressionStrategy::Zstd { level } => {
                zstd::encode_all(&serialized[..], level)?
            }
            CompressionStrategy::Lz4 { acceleration } => {
                lz4::compress(&serialized, acceleration)?
            }
            CompressionStrategy::None => serialized,
        };

        Ok(CompressedDiff {
            data: compressed,
            checksum: blake3::hash(&compressed),
            sequence: batch.sequence,
            operation_count: batch.operations.len(),
        })
    }

    // CLAUDE: It feels a bit confusing to have the Snapshot service live in the Compressor. The Compressor feels like it should be dedicated to compression only. Should snapshotting itself live in the batcher? Give a recommendation on data flow and implement it
    async fn create_snapshot(&self) -> Result<CompressedSnapshot> {
        // Use rusqlite backup API to create consistent snapshot
        let conn = Connection::open(&self.db_path)?;
        let backup = Backup::new(&conn, Path::new("/tmp/snapshot.db"))?;
        backup.run_to_completion(100, Duration::from_millis(100), None)?;

        // Compress the snapshot
        let snapshot_data = tokio::fs::read("/tmp/snapshot.db").await?;
        let compressed = zstd::encode_all(&snapshot_data[..], 3)?;

        Ok(CompressedSnapshot {
            data: compressed,
            checksum: blake3::hash(&compressed),
            sequence: self.current_sequence(),
            timestamp: SystemTime::now(),
        })
    }
}
```

### 4. Multi-DA Publisher

Publishes to multiple DA layers in parallel with retry logic:

```rust
// src/publish/mod.rs
pub struct Publisher {
    publishers: Vec<Box<dyn DaPublisher>>,
    retry_policy: RetryPolicy,
    manifest_store: ManifestStore,
}

#[async_trait]
pub trait DaPublisher: Send + Sync {
    async fn publish(&self, payload: &PublishPayload) -> Result<PublishReceipt>;
    fn name(&self) -> &str;
}

impl Publisher {
    pub async fn run(self, rx: Receiver<PublishPayload>) {
        while let Some(payload) = rx.recv().await {
            let results = self.publish_all(payload).await;
            self.manifest_store.record(results).await?;
        }
    }

    async fn publish_all(&self, payload: PublishPayload) -> Vec<PublishResult> {
        // Publish to all DA layers in parallel
        let futures = self.publishers.iter().map(|publisher| {
            self.publish_with_retry(publisher, &payload)
        });

        futures::future::join_all(futures).await
    }

    async fn publish_with_retry(&self, publisher: &dyn DaPublisher, payload: &PublishPayload) -> PublishResult {
        let backoff = ExponentialBackoff {
            max_elapsed_time: Some(Duration::from_secs(60)),
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

// CLAUDE: We will also need to read from key management here, to ensure that we have the keys available to sign published messages

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

The sidecar leverages GCP Confidential Space for hardware-protected key management and attestation, eliminating the need for complex SGX SDKs while providing strong security guarantees.

CLAUDE: You seem to have misinterpreted the needs for key management. GCP Confidential Space handles TEE key management automatically. What we need is a key that can be used for signing in the crypto space, that matches Ethereum's cryptography for ecrecover within Solidity. That needs to be managed securely within the TEE, and we should ensure that it is NOT extractable by the application from the sidecar. This is obviously quite delicate. As a side note, is it even possible to make this secure, or should the sidecar be in a separate VM from the application and it simply reads from e.g. the same underlying file system? I'm not sure what is the best practice here

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                  GCP Confidential Space VM                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │             synddb-sidecar Container                  │  │
│  │  ┌────────────────────────────────────────────────┐  │  │
│  │  │  Key Generation & Management                    │  │  │
│  │  │  - Generate signing key on first boot          │  │  │
│  │  │  - Store in Secret Manager via WI              │  │  │
│  │  │  - Key never leaves container memory           │  │  │
│  │  └────────────────────────────────────────────────┘  │  │
│  │  ┌────────────────────────────────────────────────┐  │  │
│  │  │  Attestation Service                           │  │  │
│  │  │  - Get attestation token from metadata         │  │  │
│  │  │  - Include code hash and measurements          │  │  │
│  │  │  - Sign published data with sealed key         │  │  │
│  │  └────────────────────────────────────────────────┘  │  │
│  └──────────────────────────────────────────────────────┘  │
│  Hardware Root of Trust (AMD SEV-SNP / Intel TDX)          │
└─────────────────────────────────────────────────────────────┘
                     ↓ Attestation Token
┌─────────────────────────────────────────────────────────────┐
│                        Bridge.sol                           │
│  - Verify attestation via SP1 zkVM                         │
│  - Register public key after verification                  │
└─────────────────────────────────────────────────────────────┘
```

### Key Management Implementation

```rust
// src/tee/confidential_space.rs
use gcp_auth::AuthenticationManager;
use google_cloud_secretmanager::client::{Client as SecretClient, ClientConfig};
use google_cloud_default::WithAuthExt;
use serde::{Deserialize, Serialize};
use ed25519_dalek::{SigningKey, VerifyingKey, Signature, Signer};
use rand::rngs::OsRng;
use anyhow::Result;

pub struct ConfidentialSpaceManager {
    signing_key: SigningKey,
    public_key: VerifyingKey,
    secret_client: SecretClient,
    project_id: String,
    secret_name: String,
}

#[derive(Serialize, Deserialize)]
struct SealedKeyData {
    private_key: Vec<u8>,
    public_key: Vec<u8>,
    created_at: i64,
    attestation_token: String,
}

impl ConfidentialSpaceManager {
    pub async fn init() -> Result<Self> {
        // Get project ID from metadata
        let project_id = Self::get_project_id().await?;

        // Initialize Secret Manager client with Workload Identity
        let config = ClientConfig::default().with_auth().await?;
        let secret_client = SecretClient::new(config).await?;

        let secret_name = format!("synddb-sidecar-signing-key");

        // Try to load existing key or generate new one
        let (signing_key, public_key) = match Self::load_sealed_key(&secret_client, &project_id, &secret_name).await {
            Ok(key_data) => {
                info!("Loaded existing signing key from Secret Manager");
                let signing_key = SigningKey::from_bytes(&key_data.private_key)?;
                let public_key = VerifyingKey::from_bytes(&key_data.public_key)?;
                (signing_key, public_key)
            }
            Err(_) => {
                info!("Generating new signing key");
                let signing_key = SigningKey::generate(&mut OsRng);
                let public_key = signing_key.verifying_key();

                // Get attestation token for this container
                let attestation_token = Self::get_attestation_token().await?;

                // Seal to Secret Manager
                Self::seal_key(
                    &secret_client,
                    &project_id,
                    &secret_name,
                    &signing_key,
                    &public_key,
                    &attestation_token
                ).await?;

                (signing_key, public_key)
            }
        };

        Ok(Self {
            signing_key,
            public_key,
            secret_client,
            project_id,
            secret_name,
        })
    }

    async fn get_attestation_token() -> Result<String> {
        // Get attestation token from Confidential Space metadata service
        let client = reqwest::Client::new();

        // Custom audience for our application
        let audience = "https://synddb.io/sidecar";

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
        public_key: &VerifyingKey,
        attestation_token: &str,
    ) -> Result<()> {
        let key_data = SealedKeyData {
            private_key: signing_key.to_bytes().to_vec(),
            public_key: public_key.to_bytes().to_vec(),
            created_at: chrono::Utc::now().timestamp(),
            attestation_token: attestation_token.to_string(),
        };

        let secret_data = serde_json::to_vec(&key_data)?;

        // Create secret with Workload Identity binding
        // Only this specific container with matching attestation can access
        secret_client
            .create_secret(
                project_id,
                secret_name,
                secret_data,
                Some(vec![
                    ("synddb/environment", "confidential-space"),
                    ("synddb/component", "sidecar"),
                ]),
            )
            .await?;

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

    pub fn sign_data(&self, data: &[u8]) -> Signature {
        self.signing_key.sign(data)
    }

    pub fn public_key(&self) -> &VerifyingKey {
        &self.public_key
    }

    pub async fn get_attestation_for_data(&self, data: &[u8]) -> Result<AttestationBundle> {
        // Get fresh attestation token
        let attestation_token = Self::get_attestation_token().await?;

        // Sign the data
        let signature = self.sign_data(data);

        // Parse attestation token to extract measurements
        let token_parts: Vec<&str> = attestation_token.split('.').collect();
        let payload = base64::decode_config(token_parts[1], base64::URL_SAFE_NO_PAD)?;
        let claims: serde_json::Value = serde_json::from_slice(&payload)?;

        Ok(AttestationBundle {
            attestation_token,
            signature: signature.to_bytes().to_vec(),
            public_key: self.public_key.to_bytes().to_vec(),
            data_hash: blake3::hash(data).to_hex().to_string(),
            container_image_digest: claims["image_digest"].as_str().unwrap_or("").to_string(),
            measured_boot_hash: claims["measured_boot"].as_str().unwrap_or("").to_string(),
        })
    }
}

#[derive(Serialize, Deserialize)]
pub struct AttestationBundle {
    pub attestation_token: String,
    pub signature: Vec<u8>,
    pub public_key: Vec<u8>,
    pub data_hash: String,
    pub container_image_digest: String,
    pub measured_boot_hash: String,
}

// Integration with publisher
impl Publisher {
    pub async fn publish_with_attestation(
        &self,
        payload: PublishPayload,
        attestor: &ConfidentialSpaceManager,
    ) -> Result<Vec<PublishResult>> {
        // Get attestation for the payload
        let attestation = attestor.get_attestation_for_data(&payload.data).await?;

        // Attach attestation to payload
        let mut attested_payload = payload;
        attested_payload.attestation = Some(attestation);

        // Publish to all DA layers
        self.publish_all(attested_payload).await
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
COPY --from=builder /app/target/release/synddb-sidecar /usr/local/bin/

# Create non-root user
RUN useradd -m -u 1000 synddb && \
    chown -R synddb:synddb /usr/local/bin/synddb-sidecar

USER synddb

# Health check
HEALTHCHECK --interval=30s --timeout=3s \
    CMD curl -f http://localhost:9090/health || exit 1

# Entry point with attestation initialization
ENTRYPOINT ["/usr/local/bin/synddb-sidecar"]
CMD ["--attestation", "confidential-space", "--config", "/config/sidecar.yaml"]
```

### Deployment Configuration

```yaml
# confidential-space-deployment.yaml
apiVersion: compute.cnrm.cloud.google.com/v1beta1
kind: ComputeInstance
metadata:
  name: synddb-sidecar-tee
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
    - email: synddb-sidecar@${PROJECT_ID}.iam.gserviceaccount.com
      scopes:
        - https://www.googleapis.com/auth/cloud-platform

  metadata:
    items:
      - key: tee-container-log-redirect
        value: "true"
      - key: tee-image-reference
        value: "gcr.io/${PROJECT_ID}/synddb-sidecar:latest"
      - key: tee-restart-policy
        value: "Always"
      - key: tee-env-ATTESTATION_AUDIENCE
        value: "https://synddb.io/sidecar"
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
  name: synddb-sidecar
spec:
  displayName: SyndDB Sidecar Service Account
---
apiVersion: iam.cnrm.cloud.google.com/v1beta1
kind: IAMPolicyMember
metadata:
  name: synddb-sidecar-secretmanager
spec:
  memberFrom:
    serviceAccountRef:
      name: synddb-sidecar
  role: roles/secretmanager.secretAccessor
  resourceRef:
    kind: Project
---
apiVersion: iam.cnrm.cloud.google.com/v1beta1
kind: IAMPolicy
metadata:
  name: synddb-sidecar-secret-policy
spec:
  resourceRef:
    kind: Secret
    name: synddb-sidecar-signing-key
  policy:
    bindings:
      - role: roles/secretmanager.secretAccessor
        members:
          - serviceAccount:synddb-sidecar@${PROJECT_ID}.iam.gserviceaccount.com
        condition:
          title: Only from Confidential Space
          expression: |
            assertion.sub == 'synddb-sidecar@${PROJECT_ID}.iam.gserviceaccount.com' &&
            'image_digest' in assertion &&
            assertion.image_digest == '${EXPECTED_IMAGE_DIGEST}'
```

### Configuration File

```yaml
# config/sidecar-confidential.yaml
database:
  path: "/data/app.db"
  wal_mode: true

attestation:
  enabled: true
  provider: "gcp-confidential-space"

  # GCP Confidential Space settings
  gcp:
    project_id: "${PROJECT_ID}"
    secret_name: "synddb-sidecar-signing-key"
    attestation_audience: "https://synddb.io/sidecar"

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

    // Start sidecar
    let sidecar = Sidecar::new(config).start().await;

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
COPY --from=builder /app/target/release/synddb-sidecar /usr/local/bin/
ENTRYPOINT ["synddb-sidecar"]
```

### Resource Requirements

- **CPU**: 2 cores minimum (4 recommended)
- **Memory**: 512MB minimum (2GB recommended)
- **Disk**: 10GB for caching
- **Network**: 100Mbps for DA publishing

### Monitoring

Prometheus metrics exposed on port 9090:

- `synddb_wal_frames_processed`
- `synddb_batches_published`
- `synddb_da_publish_latency`
- `synddb_compression_ratio`

## Security Considerations

### 1. Read-Only Access

Sidecar only reads from SQLite, never writes:

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

### 3. TEE Isolation

When running in TEE, keys never leave enclave:

```rust
let sealed_key = enclave.seal_data(&signing_key)?;
// Key is sealed to this specific enclave
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

2. **Deploy sidecar**:

```bash
docker run -v /app/data:/data syndicate/synddb-sidecar
```

3. **Verify publishing**:

```bash
curl http://localhost:9090/metrics | grep synddb_
```

No application code changes required - the sidecar is completely passive and transparent to the application.
