# PLAN_REPLICA.md - Read Replica and Validator Implementation

## Overview

The synddb-replica serves dual purposes: as a permissionless read replica that syncs from DA layers and serves queries, and as a validator when run in TEE mode with settlement capabilities. The same binary operates in different modes based on configuration, providing a unified codebase for both read serving and validation.

**Key Integration Points:**
- Consumes `SignedMessage` from sequencer's DA publishers (GCS, Celestia, etc.)
- Applies SQLite changesets (binary format from Session Extension), not SQL statements
- Verifies sequencer signatures using secp256k1 (same scheme as sequencer)
- Reuses `synddb-chain-monitor` for blockchain event handling

## Architecture

```
┌────────────────────────────────────────────────────────────────┐
│                         DA Layers                              │
│  ┌──────────┐ ┌──────────┐ ┌──────┐ ┌──────────┐            │
│  │   GCS    │ │ Celestia │ │ IPFS │ │ EigenDA  │            │
│  └──────────┘ └──────────┘ └──────┘ └──────────┘            │
└────────────────────────────────────────────────────────────────┘
                    ↓                ↓
┌────────────────────────────────────────────────────────────────┐
│                    synddb-replica                              │
│ ┌──────────────────────────────────────────────────────────┐  │
│ │                    DA Syncer                              │  │
│ │  ┌────────────┐  ┌────────────┐  ┌────────────┐        │  │
│ │  │ Fetcher    │→ │ Verifier   │→ │  Orderer   │        │  │
│ │  │(GCS/DA)    │  │(Signature) │  │ (Sequence) │        │  │
│ │  └────────────┘  └────────────┘  └────────────┘        │  │
│ └──────────────────────────────────────────────────────────┘  │
│                           ↓                                    │
│ ┌──────────────────────────────────────────────────────────┐  │
│ │               Changeset Applier                           │  │
│ │  ┌────────────┐  ┌────────────┐  ┌────────────┐        │  │
│ │  │Decompress  │→ │  Apply     │→ │ Validate   │        │  │
│ │  │  (zstd)    │  │ (Session)  │  │(Invariants)│        │  │
│ │  └────────────┘  └────────────┘  └────────────┘        │  │
│ └──────────────────────────────────────────────────────────┘  │
│                           ↓                                    │
│        ┌──────────────────────────────────────┐                   │
│        │        Local SQLite DB           │                   │
│        └──────────────────────────────────┘                   │
│                    ↓              ↓                            │
│ ┌─────────────────────┐  ┌──────────────────────────────┐    │
│ │   Query Server      │  │   Validator Mode (TEE)       │    │
│ │  ┌──────────────┐  │  │  ┌────────────────────────┐  │    │
│ │  │  JSON-RPC    │  │  │  │ Chain Monitor          │  │    │
│ │  └──────────────┘  │  │  │ (synddb-chain-monitor) │  │    │
│ │  ┌──────────────┐  │  │  └────────────────────────┘  │    │
│ │  │  REST API    │  │  │  ┌────────────────────────┐  │    │
│ │  └──────────────┘  │  │  │ Settlement Poster       │  │    │
│ │  ┌──────────────┐  │  │  └────────────────────────┘  │    │
│ │  │  WebSocket   │  │  │  ┌────────────────────────┐  │    │
│ │  └──────────────┘  │  │  │ TEE Attestation        │  │    │
│ └─────────────────────┘  └──────────────────────────┘  │    │
└────────────────────────────────────────────────────────────────┘
```

## Data Formats (Aligned with Sequencer)

### SignedMessage (from sequencer)

The replica fetches `SignedMessage` objects from DA layers. This is the exact format produced by `synddb-sequencer`:

```rust
/// A message that has been sequenced and signed by the sequencer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedMessage {
    /// Monotonically increasing sequence number (assigned by sequencer)
    pub sequence: u64,
    /// Unix timestamp (seconds) when sequenced
    pub timestamp: u64,
    /// Type of message: "changeset", "withdrawal", or "snapshot"
    pub message_type: MessageType,
    /// zstd-compressed JSON payload
    pub payload: Vec<u8>,
    /// keccak256 hash of compressed payload (hex with 0x prefix)
    pub message_hash: String,
    /// secp256k1 signature over (sequence || timestamp || message_hash)
    pub signature: String,
    /// Ethereum address of sequencer (hex with 0x prefix)
    pub signer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Changeset,
    Withdrawal,
    Snapshot,
}
```

### Payload Formats (after zstd decompression)

**Changeset Batch** (MessageType::Changeset):
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesetBatchRequest {
    pub batch_id: String,
    pub changesets: Vec<ChangesetData>,
    pub attestation_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesetData {
    /// Raw SQLite changeset bytes (base64 encoded)
    #[serde(with = "base64_serde")]
    pub data: Vec<u8>,
    /// Client-side sequence number
    pub sequence: u64,
    /// Client-side timestamp
    pub timestamp: u64,
}
```

**Snapshot** (MessageType::Snapshot):
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRequest {
    pub message_id: String,
    pub snapshot: SnapshotData,
    pub attestation_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotData {
    /// Complete SQLite database file (base64 encoded)
    #[serde(with = "base64_serde")]
    pub data: Vec<u8>,
    pub timestamp: u64,
    /// Sequence number - changesets >= this apply after snapshot
    pub sequence: u64,
}
```

**Withdrawal** (MessageType::Withdrawal):
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithdrawalRequest {
    pub request_id: String,
    pub recipient: String,  // Ethereum address (0x...)
    pub amount: String,     // Decimal string
    #[serde(with = "base64_serde")]
    pub data: Vec<u8>,      // Optional calldata
}
```

## Core Libraries

```toml
[dependencies]
# Workspace crates (reuse existing implementations)
synddb-chain-monitor = { path = "../synddb-chain-monitor" }

# SQLite with Session Extension for changeset application
rusqlite = { version = "0.32", features = ["bundled", "backup", "session", "hooks"] }

# Compression (matches sequencer's zstd compression)
zstd = "0.13"

# DA Layer clients
google-cloud-storage = "0.20"  # GCS (primary, matches sequencer)
celestia-rpc = "0.6"           # Celestia DA
# eigenda and others added as features

# Blockchain interaction (matches sequencer)
alloy = { version = "0.9", features = ["full", "signer-local"] }

# Async runtime
tokio = { version = "1.35", features = ["full"] }
futures = "0.3"
async-trait = "0.1"

# API servers
axum = { version = "0.7", features = ["ws"] }  # REST and WebSocket
jsonrpsee = { version = "0.24", features = ["server", "macros"] }  # JSON-RPC
tower = "0.5"
tower-http = { version = "0.6", features = ["cors", "trace"] }

# Serialization (matches sequencer format)
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
base64 = "0.22"
hex = "0.4"

# Cryptography for signature verification
k256 = { version = "0.13", features = ["ecdsa"] }

# Configuration (matches project patterns)
clap = { version = "4.4", features = ["derive", "env"] }
humantime-serde = "1.1"

# Error handling
anyhow = "1.0"
thiserror = "1.0"

# Logging (matches project patterns)
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# Utilities
dashmap = "6.1"
crossbeam-channel = "0.5"
uuid = { version = "1.0", features = ["v4"] }
reqwest = { version = "0.12", features = ["json"] }

[features]
default = ["gcs"]
gcs = ["google-cloud-storage"]
celestia = ["celestia-rpc"]
eigenda = []
confidential-space = []  # GCP Confidential Space TEE
```

## Directory Structure

```
synddb-replica/
├── Cargo.toml
├── src/
│   ├── main.rs                      # Entry point
│   ├── lib.rs                       # Public API
│   ├── config.rs                    # Configuration (clap + env vars)
│   ├── sync/
│   │   ├── mod.rs                   # DA syncing orchestration
│   │   ├── fetcher.rs               # Fetch SignedMessage from DA
│   │   ├── verifier.rs              # Verify sequencer signatures
│   │   ├── state_manager.rs         # Track sync state (SQLite)
│   │   └── providers/
│   │       ├── mod.rs               # DAFetcher trait
│   │       ├── gcs.rs               # GCS fetcher (primary)
│   │       ├── celestia.rs          # Celestia fetcher
│   │       └── mock.rs              # Mock for testing
│   ├── apply/
│   │   ├── mod.rs                   # Changeset application engine
│   │   ├── applier.rs               # Apply SQLite changesets
│   │   ├── snapshot.rs              # Restore from snapshots
│   │   ├── invariants.rs            # Post-apply invariant checks
│   │   └── types.rs                 # Shared types
│   ├── database/
│   │   ├── mod.rs                   # SQLite management
│   │   ├── pool.rs                  # Read connection pool
│   │   └── state.rs                 # Replica state tracking
│   ├── api/
│   │   ├── mod.rs                   # API servers
│   │   ├── rest.rs                  # REST API (axum)
│   │   ├── jsonrpc.rs               # JSON-RPC server
│   │   └── websocket.rs             # WebSocket subscriptions
│   ├── validator/
│   │   ├── mod.rs                   # Validator mode
│   │   ├── withdrawal_handler.rs    # Process withdrawal messages
│   │   ├── settlement.rs            # Post to bridge contract
│   │   └── consensus.rs             # Multi-validator coordination
│   ├── tee/
│   │   ├── mod.rs                   # TEE integration
│   │   ├── confidential_space.rs    # GCP Confidential Space
│   │   └── attestation.rs           # Generate/verify attestations
│   └── metrics.rs                   # Prometheus metrics
├── tests/
│   ├── integration/
│   │   ├── sync_test.rs             # End-to-end sync tests
│   │   └── apply_test.rs            # Changeset application tests
│   └── fixtures/                    # Test data
└── README.md
```

## Core Components

### 1. DA Syncer

Fetches `SignedMessage` from DA layers and verifies sequencer signatures. The trait mirrors the sequencer's `DAPublisher` interface for consistency:

```rust
// src/sync/providers/mod.rs

use crate::SignedMessage;

/// Trait for fetching messages from DA layers
/// Mirrors sequencer's DAPublisher trait for consistency
#[async_trait]
pub trait DAFetcher: Send + Sync + std::fmt::Debug {
    /// Name of this fetcher (e.g., "gcs", "celestia")
    fn name(&self) -> &str;

    /// Fetch a signed message by sequence number
    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>>;

    /// Get the latest sequence number available
    async fn get_latest_sequence(&self) -> Result<Option<u64>>;
}

// src/sync/mod.rs
pub struct DaSyncer {
    providers: Vec<Arc<dyn DAFetcher>>,
    state_manager: StateManager,
    verifier: SignatureVerifier,
    expected_signer: Address,
}

impl DaSyncer {
    pub fn new(
        providers: Vec<Arc<dyn DAFetcher>>,
        state_manager: StateManager,
        expected_signer: Address,
    ) -> Self {
        Self {
            providers,
            state_manager,
            verifier: SignatureVerifier::new(),
            expected_signer,
        }
    }

    pub async fn run(&self, tx: Sender<SignedMessage>) -> Result<()> {
        info!("DA Syncer started, expected signer: {:?}", self.expected_signer);

        loop {
            let latest_sequence = self.get_latest_sequence().await?;
            let local_sequence = self.state_manager.get_sequence()?;

            if let Some(latest) = latest_sequence {
                if latest > local_sequence {
                    info!(
                        local = local_sequence,
                        remote = latest,
                        behind = latest - local_sequence,
                        "Syncing messages"
                    );

                    // Fetch and verify each message in order
                    for seq in (local_sequence + 1)..=latest {
                        match self.fetch_and_verify(seq).await {
                            Ok(msg) => {
                                tx.send(msg).await
                                    .context("Failed to send message to applier")?;
                                self.state_manager.update_sequence(seq)?;
                            }
                            Err(e) => {
                                error!(sequence = seq, error = %e, "Failed to fetch/verify message");
                                break; // Stop at first failure to maintain ordering
                            }
                        }
                    }
                }
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn fetch_and_verify(&self, sequence: u64) -> Result<SignedMessage> {
        // Try each provider until one succeeds
        for provider in &self.providers {
            match provider.get(sequence).await {
                Ok(Some(msg)) => {
                    // Verify signature matches expected signer
                    self.verifier.verify(&msg, self.expected_signer)?;
                    return Ok(msg);
                }
                Ok(None) => continue,
                Err(e) => {
                    warn!(provider = provider.name(), error = %e, "Provider fetch failed");
                    continue;
                }
            }
        }

        Err(anyhow!("Failed to fetch sequence {} from all providers", sequence))
    }

    async fn get_latest_sequence(&self) -> Result<Option<u64>> {
        for provider in &self.providers {
            if let Ok(seq) = provider.get_latest_sequence().await {
                return Ok(seq);
            }
        }
        Ok(None)
    }
}

// src/sync/providers/gcs.rs
// GCS fetcher - reads from same format as sequencer's GcsPublisher

use google_cloud_storage::client::{Client, ClientConfig};
use google_cloud_default::WithAuthExt;

pub struct GcsFetcher {
    client: Client,
    bucket: String,
    prefix: String,
}

impl GcsFetcher {
    pub async fn new(bucket: String, prefix: String) -> Result<Self> {
        let config = ClientConfig::default().with_auth().await
            .context("Failed to configure GCS auth")?;
        let client = Client::new(config);

        info!(bucket = %bucket, prefix = %prefix, "GCS fetcher initialized");
        Ok(Self { client, bucket, prefix })
    }

    fn message_path(&self, sequence: u64) -> String {
        // Matches sequencer's GcsPublisher format: {prefix}/messages/{sequence:012}.json
        format!("{}/messages/{:012}.json", self.prefix, sequence)
    }
}

#[async_trait]
impl DAFetcher for GcsFetcher {
    fn name(&self) -> &str {
        "gcs"
    }

    async fn get(&self, sequence: u64) -> Result<Option<SignedMessage>> {
        use google_cloud_storage::http::objects::download::Range;
        use google_cloud_storage::http::objects::get::GetObjectRequest;

        let path = self.message_path(sequence);
        let request = GetObjectRequest {
            bucket: self.bucket.clone(),
            object: path.clone(),
            ..Default::default()
        };

        match self.client.download_object(&request, &Range::default()).await {
            Ok(data) => {
                let message: SignedMessage = serde_json::from_slice(&data)
                    .context("Failed to parse SignedMessage")?;
                Ok(Some(message))
            }
            Err(e) => {
                let error_str = e.to_string();
                if error_str.contains("404") || error_str.contains("No such object") {
                    Ok(None)
                } else {
                    Err(anyhow!("GCS download error: {}", e))
                }
            }
        }
    }

    async fn get_latest_sequence(&self) -> Result<Option<u64>> {
        use google_cloud_storage::http::objects::list::ListObjectsRequest;

        let prefix = format!("{}/messages/", self.prefix);
        let request = ListObjectsRequest {
            bucket: self.bucket.clone(),
            prefix: Some(prefix),
            ..Default::default()
        };

        let response = self.client.list_objects(&request).await
            .context("Failed to list GCS objects")?;

        let max_seq = response
            .items
            .unwrap_or_default()
            .iter()
            .filter_map(|obj| {
                obj.name
                    .rsplit('/')
                    .next()
                    .and_then(|filename| filename.strip_suffix(".json"))
                    .and_then(|seq_str| seq_str.parse::<u64>().ok())
            })
            .max();

        Ok(max_seq)
    }
}
```

### 2. Signature Verifier

Verifies sequencer signatures using the same scheme as `synddb-sequencer`:

```rust
// src/sync/verifier.rs

use alloy::primitives::{keccak256, Address, B256};
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

/// Verifies sequencer signatures on SignedMessage
pub struct SignatureVerifier;

impl SignatureVerifier {
    pub fn new() -> Self {
        Self
    }

    /// Verify that a SignedMessage was signed by the expected signer
    ///
    /// The sequencer signs: keccak256(sequence || timestamp || message_hash)
    pub fn verify(&self, message: &SignedMessage, expected_signer: Address) -> Result<()> {
        // Reconstruct the message that was signed
        // Sequencer signs: (sequence || timestamp || message_hash)
        let mut signing_data = Vec::new();
        signing_data.extend_from_slice(&message.sequence.to_be_bytes());
        signing_data.extend_from_slice(&message.timestamp.to_be_bytes());

        // message_hash is hex string with 0x prefix
        let hash_bytes = hex::decode(message.message_hash.strip_prefix("0x").unwrap_or(&message.message_hash))
            .context("Invalid message_hash hex")?;
        signing_data.extend_from_slice(&hash_bytes);

        let message_hash = keccak256(&signing_data);

        // Parse signature (hex string with 0x prefix)
        let sig_bytes = hex::decode(message.signature.strip_prefix("0x").unwrap_or(&message.signature))
            .context("Invalid signature hex")?;

        if sig_bytes.len() != 65 {
            return Err(anyhow!("Invalid signature length: expected 65, got {}", sig_bytes.len()));
        }

        // Split into r, s, v components
        let r = &sig_bytes[0..32];
        let s = &sig_bytes[32..64];
        let v = sig_bytes[64];

        // Convert v to recovery id (Ethereum uses 27/28, ECDSA uses 0/1)
        let recovery_id = RecoveryId::try_from(if v >= 27 { v - 27 } else { v })
            .context("Invalid recovery id")?;

        // Reconstruct signature
        let signature = Signature::from_slice(&sig_bytes[0..64])
            .context("Invalid signature bytes")?;

        // Recover public key
        let recovered_key = VerifyingKey::recover_from_prehash(
            message_hash.as_slice(),
            &signature,
            recovery_id,
        ).context("Failed to recover public key")?;

        // Convert to Ethereum address
        let recovered_address = public_key_to_address(&recovered_key);

        // Parse expected signer address
        let expected: Address = message.signer.parse()
            .context("Invalid signer address in message")?;

        if recovered_address != expected {
            return Err(anyhow!(
                "Recovered address {} does not match signer {}",
                recovered_address, expected
            ));
        }

        if recovered_address != expected_signer {
            return Err(anyhow!(
                "Signer {} is not the expected sequencer {}",
                recovered_address, expected_signer
            ));
        }

        // Also verify the message_hash matches the payload
        let computed_hash = keccak256(&message.payload);
        let computed_hash_hex = format!("0x{}", hex::encode(computed_hash));

        if computed_hash_hex != message.message_hash {
            return Err(anyhow!(
                "Payload hash mismatch: computed {}, expected {}",
                computed_hash_hex, message.message_hash
            ));
        }

        Ok(())
    }
}

fn public_key_to_address(key: &VerifyingKey) -> Address {
    let public_key_bytes = key.to_encoded_point(false);
    let hash = keccak256(&public_key_bytes.as_bytes()[1..]); // Skip 0x04 prefix
    Address::from_slice(&hash[12..])
}
```

### 3. Changeset Applier

Applies SQLite changesets from the sequencer. This replaces the SQL replay approach - we use rusqlite's Session Extension to apply binary changesets directly:

```rust
// src/apply/mod.rs

use rusqlite::Connection;
use std::io::Read;

/// Applies changesets and snapshots to the local database
pub struct ChangesetApplier {
    conn: Connection,
    invariant_checker: Option<InvariantChecker>,
}

impl ChangesetApplier {
    pub fn new(db_path: &str, invariant_checker: Option<InvariantChecker>) -> Result<Self> {
        let conn = Connection::open(db_path)?;

        // Enable WAL mode for better concurrency
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

        Ok(Self { conn, invariant_checker })
    }

    /// Main loop: receive messages and apply them
    pub async fn run(&mut self, rx: Receiver<SignedMessage>) -> Result<()> {
        info!("Changeset Applier started");

        while let Ok(message) = rx.recv().await {
            match self.apply_message(&message) {
                Ok(()) => {
                    debug!(
                        sequence = message.sequence,
                        message_type = ?message.message_type,
                        "Message applied successfully"
                    );
                }
                Err(e) => {
                    error!(
                        sequence = message.sequence,
                        error = %e,
                        "Failed to apply message"
                    );
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    fn apply_message(&mut self, message: &SignedMessage) -> Result<()> {
        // Decompress payload (zstd)
        let decompressed = self.decompress_payload(&message.payload)?;

        match message.message_type {
            MessageType::Changeset => self.apply_changeset_batch(&decompressed),
            MessageType::Snapshot => self.apply_snapshot(&decompressed),
            MessageType::Withdrawal => self.record_withdrawal(&decompressed),
        }
    }

    fn decompress_payload(&self, compressed: &[u8]) -> Result<Vec<u8>> {
        let mut decoder = zstd::Decoder::new(compressed)
            .context("Failed to create zstd decoder")?;
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)
            .context("Failed to decompress payload")?;
        Ok(decompressed)
    }

    fn apply_changeset_batch(&mut self, data: &[u8]) -> Result<()> {
        // Parse the ChangesetBatchRequest JSON
        let batch: ChangesetBatchRequest = serde_json::from_slice(data)
            .context("Failed to parse changeset batch")?;

        debug!(
            batch_id = %batch.batch_id,
            changesets = batch.changesets.len(),
            "Applying changeset batch"
        );

        // Apply each changeset within a transaction
        let tx = self.conn.transaction()?;

        for changeset in &batch.changesets {
            // changeset.data is base64-decoded binary SQLite changeset
            self.apply_single_changeset(&tx, &changeset.data)?;
        }

        // Check invariants before commit (if configured)
        if let Some(ref checker) = self.invariant_checker {
            checker.check(&tx)?;
        }

        tx.commit()?;
        Ok(())
    }

    fn apply_single_changeset(&self, conn: &Connection, changeset_data: &[u8]) -> Result<()> {
        // Use rusqlite's changeset_apply function
        // This applies the binary changeset directly to the database
        conn.execute_batch("SAVEPOINT changeset_apply")?;

        match rusqlite::session::Changeset::new(changeset_data) {
            Ok(changeset) => {
                // Apply with conflict resolution: abort on any conflict
                changeset.apply(conn, None::<fn(&str) -> bool>, |_conflict_type| {
                    rusqlite::session::ConflictAction::Abort
                }).context("Failed to apply changeset")?;

                conn.execute_batch("RELEASE changeset_apply")?;
                Ok(())
            }
            Err(e) => {
                conn.execute_batch("ROLLBACK TO changeset_apply")?;
                Err(anyhow!("Invalid changeset data: {}", e))
            }
        }
    }

    fn apply_snapshot(&mut self, data: &[u8]) -> Result<()> {
        // Parse the SnapshotRequest JSON
        let request: SnapshotRequest = serde_json::from_slice(data)
            .context("Failed to parse snapshot request")?;

        info!(
            message_id = %request.message_id,
            sequence = request.snapshot.sequence,
            size = request.snapshot.data.len(),
            "Applying snapshot"
        );

        // Write snapshot to temp file
        let temp_path = std::env::temp_dir()
            .join(format!("synddb_snapshot_{}.db", uuid::Uuid::new_v4()));

        std::fs::write(&temp_path, &request.snapshot.data)
            .context("Failed to write snapshot to temp file")?;

        // Restore from snapshot using SQLite backup API
        let source = Connection::open(&temp_path)?;
        let backup = rusqlite::backup::Backup::new(&source, &mut self.conn)?;

        backup.run_to_completion(5, std::time::Duration::from_millis(250), None)
            .context("Failed to restore from snapshot")?;

        // Clean up temp file
        let _ = std::fs::remove_file(&temp_path);

        info!(sequence = request.snapshot.sequence, "Snapshot applied successfully");
        Ok(())
    }

    fn record_withdrawal(&mut self, data: &[u8]) -> Result<()> {
        // Parse the WithdrawalRequest JSON
        let request: WithdrawalRequest = serde_json::from_slice(data)
            .context("Failed to parse withdrawal request")?;

        debug!(
            request_id = %request.request_id,
            recipient = %request.recipient,
            amount = %request.amount,
            "Recording withdrawal"
        );

        // Withdrawals are recorded for validators to process
        // The actual withdrawal was already applied via changesets from the client
        // This just logs the withdrawal request for validator settlement

        // In validator mode, this would be added to a pending withdrawals queue
        Ok(())
    }
}

// src/apply/invariants.rs

/// Optional post-apply invariant checks
pub struct InvariantChecker {
    checks: Vec<Box<dyn InvariantCheck>>,
}

pub trait InvariantCheck: Send + Sync {
    fn check(&self, conn: &Connection) -> Result<()>;
}

impl InvariantChecker {
    pub fn new() -> Self {
        Self { checks: Vec::new() }
    }

    pub fn add_check(&mut self, check: Box<dyn InvariantCheck>) {
        self.checks.push(check);
    }

    pub fn check(&self, conn: &Connection) -> Result<()> {
        for check in &self.checks {
            check.check(conn)?;
        }
        Ok(())
    }
}

/// Example: Check no negative balances
pub struct NoNegativeBalances {
    table: String,
    column: String,
}

impl InvariantCheck for NoNegativeBalances {
    fn check(&self, conn: &Connection) -> Result<()> {
        let sql = format!(
            "SELECT COUNT(*) FROM {} WHERE {} < 0",
            self.table, self.column
        );

        let count: i64 = conn.query_row(&sql, [], |row| row.get(0))?;

        if count > 0 {
            return Err(anyhow!(
                "Invariant violation: {} negative values in {}.{}",
                count, self.table, self.column
            ));
        }

        Ok(())
    }
}
```

### 4. Query Server

Serves queries via multiple protocols:

```rust
// src/api/jsonrpc/mod.rs
pub struct JsonRpcServer {
    db_pool: SqlitePool,
    methods: RpcMethods,
}

#[rpc(server)]
pub trait SyndDbRpc {
    #[method(name = "query")]
    async fn query(&self, sql: String) -> Result<QueryResult, Error>;
    
    #[method(name = "getStateHash")]
    async fn get_state_hash(&self) -> Result<String, Error>;
    
    #[method(name = "getSequence")]
    async fn get_sequence(&self) -> Result<u64, Error>;
    
    #[method(name = "subscribe")]
    async fn subscribe(&self, table: String) -> Result<SubscriptionId, Error>;
}

impl SyndDbRpcServer for JsonRpcServer {
    async fn query(&self, sql: String) -> Result<QueryResult, Error> {
        // Only allow SELECT queries
        if !sql.trim().to_uppercase().starts_with("SELECT") {
            return Err(Error::Custom("Only SELECT queries allowed".into()));
        }
        
        let conn = self.db_pool.get().await?;
        let rows = conn.query(&sql)?;
        
        Ok(QueryResult {
            columns: rows.columns(),
            rows: rows.to_json(),
        })
    }
}

// src/api/rest/mod.rs
pub struct RestServer {
    db_pool: SqlitePool,
}

pub fn routes(state: Arc<RestServer>) -> Router {
    Router::new()
        .route("/query", post(query_handler))
        .route("/tables", get(list_tables))
        .route("/table/:name", get(describe_table))
        .route("/health", get(health_check))
        .route("/metrics", get(prometheus_metrics))
        .with_state(state)
}

async fn query_handler(
    State(server): State<Arc<RestServer>>,
    Json(query): Json<QueryRequest>,
) -> Result<Json<QueryResponse>> {
    let result = server.execute_query(query.sql).await?;
    Ok(Json(result))
}

// src/api/websocket/mod.rs
pub struct WebSocketServer {
    db_pool: SqlitePool,
    subscriptions: Arc<DashMap<String, Vec<WebSocketSink>>>,
}

impl WebSocketServer {
    pub async fn handle_connection(&self, ws: WebSocket) {
        let (mut sender, mut receiver) = ws.split();
        
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    let request: WsRequest = serde_json::from_str(&text)?;
                    self.handle_request(request, &mut sender).await?;
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    }
    
    async fn handle_request(&self, req: WsRequest, sender: &mut WebSocketSink) {
        match req {
            WsRequest::Subscribe { table } => {
                self.subscriptions.entry(table).or_default().push(sender.clone());
            }
            WsRequest::Query { sql } => {
                let result = self.execute_query(sql).await?;
                sender.send(Message::Text(serde_json::to_string(&result)?)).await?;
            }
        }
    }
}
```

### 5. Validator Mode

Validators process withdrawal messages and post settlements to L1. This mode reuses `synddb-chain-monitor` for blockchain event handling:

```rust
// src/validator/mod.rs

use synddb_chain_monitor::{ChainMonitor, ChainMonitorConfig, MessageHandler};

/// Validator mode adds withdrawal processing and settlement posting
pub struct ValidatorMode {
    withdrawal_handler: WithdrawalHandler,
    chain_monitor: ChainMonitor,
    settlement_poster: SettlementPoster,
    pending_withdrawals: Arc<DashMap<String, PendingWithdrawal>>,
}

impl ValidatorMode {
    pub async fn new(
        config: ValidatorConfig,
        db_conn: Arc<Mutex<Connection>>,
    ) -> Result<Self> {
        // Create withdrawal handler (implements MessageHandler trait)
        let pending_withdrawals = Arc::new(DashMap::new());
        let withdrawal_handler = WithdrawalHandler {
            pending_withdrawals: pending_withdrawals.clone(),
        };

        // Create chain monitor using synddb-chain-monitor crate
        let monitor_config = ChainMonitorConfig {
            ws_urls: config.chain_ws_urls,
            contract_address: config.bridge_contract,
            start_block: config.start_block,
            event_store_path: config.event_store_path,
            ..Default::default()
        };

        let chain_monitor = ChainMonitor::new(
            monitor_config,
            Arc::new(withdrawal_handler.clone()),
        ).await?;

        // Create settlement poster
        let settlement_poster = SettlementPoster::new(
            config.bridge_contract,
            config.rpc_url,
            config.signing_key,
        ).await?;

        Ok(Self {
            withdrawal_handler,
            chain_monitor,
            settlement_poster,
            pending_withdrawals,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("Validator mode started");

        tokio::select! {
            result = self.chain_monitor.run() => {
                error!("Chain monitor exited: {:?}", result);
                result
            }
            result = self.process_pending_withdrawals() => {
                error!("Withdrawal processor exited: {:?}", result);
                result
            }
        }
    }

    async fn process_pending_withdrawals(&self) -> Result<()> {
        loop {
            // Process any pending withdrawals that have been confirmed
            let ready: Vec<_> = self.pending_withdrawals
                .iter()
                .filter(|w| w.confirmations >= REQUIRED_CONFIRMATIONS)
                .map(|w| w.key().clone())
                .collect();

            for request_id in ready {
                if let Some((_, withdrawal)) = self.pending_withdrawals.remove(&request_id) {
                    match self.settlement_poster.post_withdrawal(&withdrawal).await {
                        Ok(tx_hash) => {
                            info!(
                                request_id = %request_id,
                                tx_hash = %tx_hash,
                                "Withdrawal posted to L1"
                            );
                        }
                        Err(e) => {
                            error!(request_id = %request_id, error = %e, "Failed to post withdrawal");
                            // Re-add for retry
                            self.pending_withdrawals.insert(request_id, withdrawal);
                        }
                    }
                }
            }

            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }
}

// src/validator/withdrawal_handler.rs

use synddb_chain_monitor::handler::MessageHandler;
use alloy::rpc::types::Log;

/// Handles withdrawal events from the sequencer
/// Implements MessageHandler trait from synddb-chain-monitor
#[derive(Debug, Clone)]
pub struct WithdrawalHandler {
    pending_withdrawals: Arc<DashMap<String, PendingWithdrawal>>,
}

#[derive(Debug, Clone)]
pub struct PendingWithdrawal {
    pub request_id: String,
    pub recipient: Address,
    pub amount: U256,
    pub data: Vec<u8>,
    pub sequence: u64,
    pub confirmations: u64,
}

#[async_trait]
impl MessageHandler for WithdrawalHandler {
    async fn handle_event(&self, log: &Log) -> Result<bool> {
        // Parse withdrawal event from sequencer
        // This would decode the specific event signature
        if let Some(withdrawal) = self.parse_withdrawal_event(log)? {
            info!(
                request_id = %withdrawal.request_id,
                recipient = %withdrawal.recipient,
                amount = %withdrawal.amount,
                "Received withdrawal event"
            );

            self.pending_withdrawals.insert(
                withdrawal.request_id.clone(),
                withdrawal,
            );

            return Ok(true);
        }

        Ok(false)
    }

    fn event_signature(&self) -> Option<alloy::primitives::B256> {
        // Return the WithdrawalRequested event signature
        // sol! { event WithdrawalRequested(bytes32 indexed requestId, address recipient, uint256 amount); }
        Some(alloy::sol_types::sol_event_signature!(
            "WithdrawalRequested(bytes32,address,uint256)"
        ))
    }
}

// src/validator/settlement.rs

use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;

pub struct SettlementPoster {
    provider: Arc<dyn Provider>,
    signer: PrivateKeySigner,
    bridge_address: Address,
}

impl SettlementPoster {
    pub async fn new(
        bridge_address: Address,
        rpc_url: String,
        signing_key: String,
    ) -> Result<Self> {
        let signer: PrivateKeySigner = signing_key.parse()
            .context("Invalid signing key")?;

        let provider = ProviderBuilder::new()
            .on_http(rpc_url.parse()?)
            .boxed();

        Ok(Self {
            provider: Arc::new(provider),
            signer,
            bridge_address,
        })
    }

    pub async fn post_withdrawal(&self, withdrawal: &PendingWithdrawal) -> Result<B256> {
        // Build the transaction to call bridge.processWithdrawal(...)
        // This would use alloy's contract bindings

        info!(
            request_id = %withdrawal.request_id,
            recipient = %withdrawal.recipient,
            amount = %withdrawal.amount,
            "Posting withdrawal to bridge contract"
        );

        // Placeholder - actual implementation would call the bridge contract
        todo!("Implement bridge contract interaction")
    }
}
```

### 6. Extension System

Allow custom validation logic (simplified from original - focuses on withdrawal validation):

```rust
// src/validator/extensions.rs

#[async_trait]
pub trait WithdrawalValidator: Send + Sync {
    /// Validate a withdrawal before it's posted to L1
    async fn validate(&self, withdrawal: &PendingWithdrawal) -> Result<()>;
}

/// Rate limit withdrawals per address
pub struct WithdrawalRateLimiter {
    daily_limit: U256,
    limits: Arc<DashMap<Address, DailyLimit>>,
}

impl WithdrawalValidator for WithdrawalRateLimiter {
    async fn validate(&self, withdrawal: &PendingWithdrawal) -> Result<()> {
        let today = chrono::Utc::now().date_naive();

        let mut entry = self.limits
            .entry(withdrawal.recipient)
            .or_insert_with(|| DailyLimit {
                date: today,
                total: U256::ZERO,
            });

        // Reset if new day
        if entry.date != today {
            entry.date = today;
            entry.total = U256::ZERO;
        }

        if entry.total + withdrawal.amount > self.daily_limit {
            return Err(anyhow!(
                "Daily withdrawal limit exceeded for {}",
                withdrawal.recipient
            ));
        }

        entry.total += withdrawal.amount;
        Ok(())
    }
}
```

## Configuration

Configuration follows the project pattern: clap derive with env var support, serde for serialization, and `humantime-serde` for durations.

### Replica Configuration (src/config.rs)

```rust
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;

/// Replica node configuration
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
#[command(name = "synddb-replica")]
#[command(about = "SyndDB Replica - syncs and serves database state from DA layers")]
pub struct ReplicaConfig {
    /// Path to the SQLite database file
    #[arg(long, env = "DATABASE_PATH", default_value = "/data/replica.db")]
    pub database_path: String,

    /// Expected sequencer address (for signature verification)
    #[arg(long, env = "SEQUENCER_ADDRESS")]
    pub sequencer_address: String,

    /// GCS bucket for fetching messages
    #[arg(long, env = "GCS_BUCKET")]
    pub gcs_bucket: Option<String>,

    /// GCS path prefix
    #[arg(long, env = "GCS_PREFIX", default_value = "sequencer")]
    pub gcs_prefix: String,

    /// Start syncing from this sequence number (0 = beginning)
    #[arg(long, env = "START_SEQUENCE", default_value = "0")]
    pub start_sequence: u64,

    /// HTTP API bind address
    #[arg(long, env = "BIND_ADDRESS", default_value = "0.0.0.0:8080")]
    pub bind_address: SocketAddr,

    /// JSON-RPC port (0 to disable)
    #[arg(long, env = "JSONRPC_PORT", default_value = "8545")]
    pub jsonrpc_port: u16,

    /// Sync poll interval
    #[arg(long, env = "SYNC_INTERVAL", default_value = "1s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub sync_interval: Duration,

    /// Request timeout
    #[arg(long, env = "REQUEST_TIMEOUT", default_value = "30s", value_parser = humantime::parse_duration)]
    #[serde(with = "humantime_serde")]
    pub request_timeout: Duration,

    /// Enable JSON log format
    #[arg(long, env = "LOG_JSON", default_value = "false")]
    pub log_json: bool,

    /// Enable validator mode
    #[arg(long, env = "VALIDATOR_MODE", default_value = "false")]
    pub validator_mode: bool,
}

impl ReplicaConfig {
    /// Create config with defaults for testing
    pub fn for_testing(db_path: &str, sequencer_address: &str) -> Self {
        let mut config = Self::parse_from(["synddb-replica", "--sequencer-address", sequencer_address]);
        config.database_path = db_path.to_string();
        config
    }
}

/// Validator-specific configuration (extends ReplicaConfig)
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
pub struct ValidatorConfig {
    /// Bridge contract address
    #[arg(long, env = "BRIDGE_CONTRACT")]
    pub bridge_contract: String,

    /// Ethereum RPC URL for settlement
    #[arg(long, env = "RPC_URL")]
    pub rpc_url: String,

    /// Chain WebSocket URLs for event monitoring
    #[arg(long, env = "CHAIN_WS_URLS", value_delimiter = ',')]
    pub chain_ws_urls: Vec<String>,

    /// Validator signing key (hex, without 0x)
    #[arg(long, env = "VALIDATOR_KEY")]
    pub signing_key: String,

    /// Start monitoring from this block
    #[arg(long, env = "START_BLOCK", default_value = "0")]
    pub start_block: u64,

    /// Path for event store (idempotency tracking)
    #[arg(long, env = "EVENT_STORE_PATH", default_value = "/data/events.db")]
    pub event_store_path: String,

    /// Minimum confirmations before processing withdrawal
    #[arg(long, env = "MIN_CONFIRMATIONS", default_value = "12")]
    pub min_confirmations: u64,
}
```

### Environment Variables

```bash
# Required
export SEQUENCER_ADDRESS="0x..."      # Sequencer's Ethereum address
export GCS_BUCKET="synddb-messages"   # GCS bucket with sequenced messages

# Optional (with defaults)
export DATABASE_PATH="/data/replica.db"
export GCS_PREFIX="sequencer"
export START_SEQUENCE="0"
export BIND_ADDRESS="0.0.0.0:8080"
export JSONRPC_PORT="8545"
export SYNC_INTERVAL="1s"
export REQUEST_TIMEOUT="30s"
export LOG_JSON="false"

# Validator mode
export VALIDATOR_MODE="true"
export BRIDGE_CONTRACT="0x..."
export RPC_URL="https://eth-mainnet.g.alchemy.com/v2/..."
export CHAIN_WS_URLS="wss://eth-mainnet.g.alchemy.com/v2/..."
export VALIDATOR_KEY="..."
export START_BLOCK="18000000"
export EVENT_STORE_PATH="/data/events.db"
export MIN_CONFIRMATIONS="12"
```

## Validator TEE Integration with GCP Confidential Space

Validators run in GCP Confidential Space to ensure secure key management and provide attestation for their signing operations. The hardware-protected environment guarantees that validator keys are generated securely and never leave the container.

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│               GCP Confidential Space Validator              │
│  ┌──────────────────────────────────────────────────────┐  │
│  │           synddb-replica (Validator Mode)             │  │
│  │  ┌────────────────────────────────────────────────┐  │  │
│  │  │  Validator Key Management                       │  │  │
│  │  │  - Generate validator keypair on init          │  │  │
│  │  │  - Store in Secret Manager with WI binding     │  │  │
│  │  │  - Keys bound to container measurements        │  │  │
│  │  └────────────────────────────────────────────────┘  │  │
│  │  ┌────────────────────────────────────────────────┐  │  │
│  │  │  Attestation & Registration                     │  │  │
│  │  │  - Generate attestation token                  │  │  │
│  │  │  - Submit to Bridge.sol with zkProof          │  │  │
│  │  │  - Register public key after verification      │  │  │
│  │  └────────────────────────────────────────────────┘  │  │
│  │  ┌────────────────────────────────────────────────┐  │  │
│  │  │  Message Signing                                │  │  │
│  │  │  - Sign withdrawal messages                    │  │  │
│  │  │  - Sign state updates                         │  │  │
│  │  │  - Include attestation proofs                  │  │  │
│  │  └────────────────────────────────────────────────┘  │  │
│  └──────────────────────────────────────────────────────┘  │
│  Hardware Root of Trust (AMD SEV-SNP / Intel TDX)          │
└─────────────────────────────────────────────────────────────┘
```

### Validator Key Management

```rust
// src/validator/confidential_validator.rs
use gcp_auth::AuthenticationManager;
use google_cloud_secretmanager::client::{Client as SecretClient, ClientConfig};
use google_cloud_default::WithAuthExt;
use k256::{ecdsa::{SigningKey as K256SigningKey, VerifyingKey as K256VerifyingKey, Signature}, SecretKey};
use alloy::signers::Signer;
use sp1_sdk::{ProverClient, SP1Stdin, SP1Proof};
use anyhow::Result;
use serde::{Serialize, Deserialize};

pub struct ConfidentialValidator {
    signing_key: K256SigningKey,
    public_key: K256VerifyingKey,
    ethereum_address: Address,
    secret_client: SecretClient,
    bridge_contract: BridgeContract,
    sp1_client: ProverClient,
    attestation_cache: Arc<RwLock<Option<ValidatorAttestation>>>,
}

#[derive(Serialize, Deserialize)]
struct ValidatorKeyData {
    private_key: Vec<u8>,
    public_key: Vec<u8>,
    ethereum_address: String,
    created_at: i64,
    initial_attestation: String,
    registered_tx_hash: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ValidatorAttestation {
    pub token: String,
    pub public_key: Vec<u8>,
    pub ethereum_address: Address,
    pub container_digest: String,
    pub measured_boot: String,
    pub timestamp: i64,
}

impl ConfidentialValidator {
    pub async fn init(bridge_contract_address: Address, rpc_url: &str) -> Result<Self> {
        let project_id = Self::get_project_id().await?;

        // Initialize Secret Manager client
        let config = ClientConfig::default().with_auth().await?;
        let secret_client = SecretClient::new(config).await?;

        // Validator-specific secret name
        let validator_id = Self::get_instance_id().await?;
        let secret_name = format!("synddb-validator-{}", validator_id);

        // Load or generate validator key
        let (signing_key, public_key, ethereum_address) =
            match Self::load_validator_key(&secret_client, &project_id, &secret_name).await {
                Ok(key_data) => {
                    info!("Loaded existing validator key");
                    let secret_key = SecretKey::from_slice(&key_data.private_key)?;
                    let signing_key = K256SigningKey::from(secret_key);
                    let public_key = signing_key.verifying_key();
                    let address = Address::from_slice(&key_data.ethereum_address);
                    (signing_key, public_key, address)
                }
                Err(_) => {
                    info!("Generating new validator key");
                    Self::generate_and_register_validator_key(
                        &secret_client,
                        &project_id,
                        &secret_name,
                        bridge_contract_address,
                        rpc_url
                    ).await?
                }
            };

        // Initialize SP1 client for zkVM proofs
        let sp1_client = ProverClient::new();

        // Connect to bridge contract
        let provider = Provider::new(Url::parse(rpc_url)?);
        let bridge_contract = BridgeContract::new(bridge_contract_address, provider);

        Ok(Self {
            signing_key,
            public_key,
            ethereum_address,
            secret_client,
            bridge_contract,
            sp1_client,
            attestation_cache: Arc::new(RwLock::new(None)),
        })
    }

    async fn generate_and_register_validator_key(
        secret_client: &SecretClient,
        project_id: &str,
        secret_name: &str,
        bridge_address: Address,
        rpc_url: &str,
    ) -> Result<(K256SigningKey, K256VerifyingKey, Address)> {
        // Generate new key
        let signing_key = K256SigningKey::random(&mut rand::thread_rng());
        let public_key = signing_key.verifying_key();
        let ethereum_address = public_key_to_address(&public_key);

        // Get attestation token
        let attestation = Self::generate_attestation(&public_key).await?;

        // Generate zkVM proof for attestation
        let zk_proof = Self::generate_attestation_proof(&attestation).await?;

        // Register with Bridge.sol
        let provider = Provider::new(Url::parse(rpc_url)?);
        let bridge = BridgeContract::new(bridge_address, provider);

        let tx = bridge
            .registerValidator(
                attestation.token.clone(),
                public_key.to_encoded_point(false).as_bytes().to_vec(),
                zk_proof,
            )
            .send()
            .await?;

        info!("Validator registered on-chain: {:?}", tx.tx_hash());

        // Seal key to Secret Manager
        let key_data = ValidatorKeyData {
            private_key: signing_key.to_bytes().to_vec(),
            public_key: public_key.to_encoded_point(false).as_bytes().to_vec(),
            ethereum_address: format!("{:?}", ethereum_address),
            created_at: chrono::Utc::now().timestamp(),
            initial_attestation: attestation.token,
            registered_tx_hash: Some(format!("{:?}", tx.tx_hash())),
        };

        secret_client
            .create_secret(
                project_id,
                secret_name,
                serde_json::to_vec(&key_data)?,
                Some(vec![
                    ("synddb/role", "validator"),
                    ("synddb/validator-id", &Self::get_instance_id().await?),
                ]),
            )
            .await?;

        Ok((signing_key, public_key, ethereum_address))
    }

    async fn generate_attestation(public_key: &K256VerifyingKey) -> Result<ValidatorAttestation> {
        // Get attestation token from metadata service
        let client = reqwest::Client::new();
        let audience = "https://synddb.io/validator";

        let response = client
            .get("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token")
            .query(&[
                ("audience", audience),
                ("format", "full"),
                ("licenses", "TRUE"),
            ])
            .header("Metadata-Flavor", "Google")
            .send()
            .await?;

        #[derive(Deserialize)]
        struct TokenResponse {
            token: String,
        }

        let token_resp: TokenResponse = response.json().await?;

        // Parse token to extract measurements
        let token_parts: Vec<&str> = token_resp.token.split('.').collect();
        let payload = base64::decode_config(token_parts[1], base64::URL_SAFE_NO_PAD)?;
        let claims: serde_json::Value = serde_json::from_slice(&payload)?;

        Ok(ValidatorAttestation {
            token: token_resp.token,
            public_key: public_key.to_encoded_point(false).as_bytes().to_vec(),
            ethereum_address: public_key_to_address(public_key),
            container_digest: claims["image_digest"].as_str().unwrap_or("").to_string(),
            measured_boot: claims["measured_boot"].as_str().unwrap_or("").to_string(),
            timestamp: chrono::Utc::now().timestamp(),
        })
    }

    async fn generate_attestation_proof(attestation: &ValidatorAttestation) -> Result<Vec<u8>> {
        // Use SP1 zkVM to generate proof of valid attestation
        let mut stdin = SP1Stdin::new();
        stdin.write(&attestation.token);
        stdin.write(&attestation.public_key);

        // Attestation verification program (pre-compiled)
        let elf = include_bytes!("../../programs/attestation-verifier/elf");

        // Generate proof
        let proof = self.sp1_client.prove(elf, stdin).await?;

        // Serialize proof for on-chain verification
        Ok(bincode::serialize(&proof)?)
    }

    pub async fn sign_message(&self, message: &Message) -> Result<ValidatorSignature> {
        // Hash the message
        let message_hash = keccak256(&abi::encode(&[
            message.id.to_token(),
            message.message_type.to_token(),
            message.schema_hash.to_token(),
            keccak256(&message.payload).to_token(),
            message.nonce.to_token(),
            message.timestamp.to_token(),
        ]));

        // Sign with Ethereum prefix
        let signature = self.signing_key.sign_message(&message_hash)?;

        // Refresh attestation if needed
        let attestation = self.refresh_attestation_if_needed().await?;

        Ok(ValidatorSignature {
            signature: signature.as_bytes().to_vec(),
            signer_address: self.ethereum_address,
            attestation_token: attestation.token,
            timestamp: chrono::Utc::now().timestamp(),
        })
    }

    async fn refresh_attestation_if_needed(&self) -> Result<ValidatorAttestation> {
        let mut cache = self.attestation_cache.write().await;

        let needs_refresh = match &*cache {
            None => true,
            Some(att) => {
                // Refresh every hour
                chrono::Utc::now().timestamp() - att.timestamp > 3600
            }
        };

        if needs_refresh {
            let new_attestation = Self::generate_attestation(&self.public_key).await?;
            *cache = Some(new_attestation.clone());
            Ok(new_attestation)
        } else {
            Ok(cache.as_ref().unwrap().clone())
        }
    }

    pub async fn sign_state_update(&self, state_update_hash: H256, sequence: u64) -> Result<StateUpdateSignature> {
        // Create state update message
        let message = StateUpdateMessage {
            state_update_hash,
            sequence,
            timestamp: chrono::Utc::now().timestamp(),
            validator: self.ethereum_address,
        };

        // Sign the message
        let message_bytes = bincode::serialize(&message)?;
        let signature = self.signing_key.sign_message(&message_bytes)?;

        // Get current attestation
        let attestation = self.refresh_attestation_if_needed().await?;

        Ok(StateUpdateSignature {
            state_update_hash,
            sequence,
            signature: signature.as_bytes().to_vec(),
            validator: self.ethereum_address,
            attestation_token: attestation.token,
        })
    }
}

#[derive(Serialize, Deserialize)]
pub struct ValidatorSignature {
    pub signature: Vec<u8>,
    pub signer_address: Address,
    pub attestation_token: String,
    pub timestamp: i64,
}

#[derive(Serialize, Deserialize)]
pub struct StateUpdateMessage {
    pub state_update_hash: H256,
    pub sequence: u64,
    pub timestamp: i64,
    pub validator: Address,
}

#[derive(Serialize, Deserialize)]
pub struct StateUpdateSignature {
    pub state_update_hash: H256,
    pub sequence: u64,
    pub signature: Vec<u8>,
    pub validator: Address,
    pub attestation_token: String,
}

fn public_key_to_address(public_key: &K256VerifyingKey) -> Address {
    let public_key_bytes = public_key.to_encoded_point(false);
    let hash = keccak256(&public_key_bytes.as_bytes()[1..]); // Skip the 0x04 prefix
    Address::from_slice(&hash[12..])
}
```

### Docker Configuration for Validators

```dockerfile
# Dockerfile.validator-confidential
FROM rust:1.75 as builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY programs ./programs

# Build with validator and TEE features
RUN cargo build --release --features "validator,confidential-space"

# Build SP1 attestation verifier program
RUN cd programs/attestation-verifier && \
    cargo prove build

# Runtime image
FROM gcr.io/confidential-space-images/base:latest

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/synddb-replica /usr/local/bin/
COPY --from=builder /app/programs/attestation-verifier/elf /usr/local/share/synddb/

# Non-root user
RUN useradd -m -u 1000 validator && \
    chown -R validator:validator /usr/local/bin/synddb-replica

USER validator

HEALTHCHECK --interval=30s --timeout=3s \
    CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/synddb-replica"]
CMD ["--mode", "validator", "--tee", "confidential-space", "--config", "/config/validator.yaml"]
```

### Deployment Configuration

```yaml
# validator-deployment.yaml
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: synddb-validators
  namespace: synddb
spec:
  serviceName: synddb-validators
  replicas: 3
  selector:
    matchLabels:
      app: synddb-validator
  template:
    metadata:
      labels:
        app: synddb-validator
    spec:
      nodeSelector:
        cloud.google.com/gke-confidential-nodes: "true"

      serviceAccountName: synddb-validator

      containers:
      - name: validator
        image: gcr.io/${PROJECT_ID}/synddb-validator:latest

        env:
        - name: PROJECT_ID
          value: "${PROJECT_ID}"
        - name: VALIDATOR_ID
          valueFrom:
            fieldRef:
              fieldPath: metadata.name
        - name: BRIDGE_CONTRACT
          value: "0x..."
        - name: RPC_URL
          valueFrom:
            secretKeyRef:
              name: synddb-config
              key: rpc-url
        - name: ATTESTATION_AUDIENCE
          value: "https://synddb.io/validator"

        ports:
        - containerPort: 8545  # JSON-RPC
        - containerPort: 8080  # REST
        - containerPort: 9090  # Metrics

        volumeMounts:
        - name: data
          mountPath: /data
        - name: config
          mountPath: /config

        resources:
          requests:
            memory: "8Gi"
            cpu: "4"
          limits:
            memory: "16Gi"
            cpu: "8"

        securityContext:
          runAsNonRoot: true
          runAsUser: 1000
          capabilities:
            drop:
            - ALL

  volumeClaimTemplates:
  - metadata:
      name: data
    spec:
      accessModes: ["ReadWriteOnce"]
      resources:
        requests:
          storage: 500Gi
```

### Configuration

```yaml
# config/validator-confidential.yaml
mode: validator

# Standard replica configuration
database:
  path: "/data/validator.db"
  max_connections: 100

sync:
  providers:
    celestia:
      enabled: true
      endpoint: "https://rpc.celestia.org"

# Validator-specific configuration
validator:
  enabled: true

  # Confidential Space TEE settings
  tee:
    provider: "gcp-confidential-space"

    gcp:
      project_id: "${PROJECT_ID}"
      validator_secret_prefix: "synddb-validator"
      attestation_audience: "https://synddb.io/validator"

      # Workload Identity configuration
      service_account: "synddb-validator@${PROJECT_ID}.iam.gserviceaccount.com"

      # Expected measurements
      expected_measurements:
        container_digest: "${EXPECTED_VALIDATOR_IMAGE_DIGEST}"

    # Attestation refresh
    attestation_refresh_mins: 60

  # Bridge contract interaction
  settlement:
    chain_id: 1
    rpc_endpoint: "${RPC_URL}"
    contract_address: "${BRIDGE_CONTRACT}"
    gas_price_multiplier: 1.2

  # Message processing
  messages:
    monitored_tables:
      - "outbound_withdrawals"
      - "outbound_messages"
    process_interval_secs: 10
    batch_size: 50

  # Coordination with other validators
  consensus:
    # Validators discover each other via k8s service
    service_name: "synddb-validators"
    namespace: "synddb"
    port: 8545

    # Minimum signatures required
    signature_threshold: 2

    # Timeout for gathering signatures
    timeout_secs: 30

  # zkVM proof generation
  zk_proof:
    enabled: true
    program_path: "/usr/local/share/synddb/attestation-verifier.elf"
    max_proof_generation_time_secs: 60

monitoring:
  metrics:
    enabled: true
    port: 9090

  health:
    enabled: true
    port: 8080
    checks:
      - attestation_validity
      - key_accessibility
      - bridge_connectivity
```

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn test_signature_verification() {
        let verifier = SignatureVerifier::new();

        // Create a test message (would need actual signed message from sequencer)
        let message = SignedMessage {
            sequence: 1,
            timestamp: 1700000000,
            message_type: MessageType::Changeset,
            payload: vec![0x01, 0x02, 0x03],
            message_hash: "0x...".to_string(),
            signature: "0x...".to_string(),
            signer: "0x742d35Cc6634C0532925a3b844Bc9e7595f2bD41".to_string(),
        };

        let expected_signer: Address = "0x742d35Cc6634C0532925a3b844Bc9e7595f2bD41".parse().unwrap();

        // Would verify against actual test vectors
        // assert!(verifier.verify(&message, expected_signer).is_ok());
    }

    #[test]
    fn test_changeset_apply() {
        // Create in-memory database with schema
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", []).unwrap();
        conn.execute("INSERT INTO users VALUES (1, 'alice')", []).unwrap();

        // Create a changeset using Session API
        let mut session = rusqlite::session::Session::new(&conn).unwrap();
        session.attach(None).unwrap();  // Attach to all tables

        // Make a change
        conn.execute("UPDATE users SET name = 'bob' WHERE id = 1", []).unwrap();

        // Get the changeset
        let changeset = session.changeset().unwrap();

        // Now apply it to another database
        let mut target = Connection::open_in_memory().unwrap();
        target.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", []).unwrap();
        target.execute("INSERT INTO users VALUES (1, 'alice')", []).unwrap();

        // Apply changeset
        let cs = rusqlite::session::Changeset::new(&changeset).unwrap();
        cs.apply(&target, None::<fn(&str) -> bool>, |_| {
            rusqlite::session::ConflictAction::Abort
        }).unwrap();

        // Verify
        let name: String = target.query_row(
            "SELECT name FROM users WHERE id = 1",
            [],
            |row| row.get(0)
        ).unwrap();
        assert_eq!(name, "bob");
    }

    #[test]
    fn test_invariant_checker() {
        let checker = NoNegativeBalances {
            table: "balances".to_string(),
            column: "amount".to_string(),
        };
        let conn = Connection::open_in_memory().unwrap();

        // Setup test data
        conn.execute("CREATE TABLE balances (account TEXT, amount INTEGER)", []).unwrap();
        conn.execute("INSERT INTO balances VALUES ('alice', -100)", []).unwrap();

        // Should fail on negative balance
        assert!(checker.check(&conn).is_err());
    }

    #[test]
    fn test_zstd_decompression() {
        let original = b"test data for compression";

        // Compress
        let compressed = zstd::encode_all(&original[..], 3).unwrap();

        // Decompress
        let decompressed = zstd::decode_all(&compressed[..]).unwrap();

        assert_eq!(&decompressed, original);
    }
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_full_sync() {
    // Start mock DA fetcher
    let mock_fetcher = Arc::new(MockDAFetcher::new());
    mock_fetcher.add_message(create_test_signed_message(1));
    mock_fetcher.add_message(create_test_signed_message(2));

    // Create replica with in-memory database
    let config = ReplicaConfig::for_testing(":memory:", "0x...");
    let (tx, rx) = tokio::sync::mpsc::channel(100);

    // Start syncer
    let state_manager = StateManager::new(":memory:").unwrap();
    let expected_signer = config.sequencer_address.parse().unwrap();
    let syncer = DaSyncer::new(vec![mock_fetcher], state_manager, expected_signer);

    // Start applier in background
    let mut applier = ChangesetApplier::new(":memory:", None).unwrap();
    let applier_handle = tokio::spawn(async move {
        applier.run(rx).await
    });

    // Run syncer briefly
    tokio::time::timeout(Duration::from_secs(2), syncer.run(tx)).await.ok();

    // Verify messages were applied
    // ...
}

fn create_test_signed_message(sequence: u64) -> SignedMessage {
    // Create a minimal test message
    SignedMessage {
        sequence,
        timestamp: chrono::Utc::now().timestamp() as u64,
        message_type: MessageType::Changeset,
        payload: zstd::encode_all(&b"{\"batch_id\":\"test\",\"changesets\":[]}"[..], 3).unwrap(),
        message_hash: "0x0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        signature: "0x".to_string() + &"00".repeat(65),
        signer: "0x0000000000000000000000000000000000000000".to_string(),
    }
}
```

### Benchmarks

```rust
use criterion::{criterion_group, criterion_main, Criterion};

fn bench_changeset_apply(c: &mut Criterion) {
    c.bench_function("apply_changeset", |b| {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)", []).unwrap();

        // Pre-create a changeset
        let changeset = create_test_changeset();

        b.iter(|| {
            let cs = rusqlite::session::Changeset::new(&changeset).unwrap();
            cs.apply(&conn, None::<fn(&str) -> bool>, |_| {
                rusqlite::session::ConflictAction::Abort
            }).unwrap();
        })
    });
}

fn bench_zstd_decompress(c: &mut Criterion) {
    // Compress 1MB of test data
    let data = vec![0u8; 1024 * 1024];
    let compressed = zstd::encode_all(&data[..], 3).unwrap();

    c.bench_function("zstd_decompress_1mb", |b| {
        b.iter(|| {
            zstd::decode_all(&compressed[..]).unwrap()
        })
    });
}
```

## Deployment

### Docker Image

```dockerfile
# Builder stage
FROM rust:1.75 as builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --features tee

# Runtime stage
FROM ubuntu:22.04
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libsgx-dcap-ql \
    libsgx-urts
    
COPY --from=builder /app/target/release/synddb-replica /usr/local/bin/
COPY config /etc/synddb/

ENTRYPOINT ["synddb-replica"]
CMD ["--config", "/etc/synddb/config.yaml"]
```

### Kubernetes Deployment

```yaml
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: synddb-replica
spec:
  serviceName: synddb-replica
  replicas: 3
  template:
    spec:
      containers:
      - name: replica
        image: syndicate/synddb-replica:latest
        ports:
        - containerPort: 8545  # JSON-RPC
        - containerPort: 8080  # REST
        - containerPort: 8546  # WebSocket
        volumeMounts:
        - name: data
          mountPath: /data
        resources:
          requests:
            memory: "4Gi"
            cpu: "2"
          limits:
            memory: "8Gi"
            cpu: "4"
  volumeClaimTemplates:
  - metadata:
      name: data
    spec:
      accessModes: ["ReadWriteOnce"]
      resources:
        requests:
          storage: 100Gi
```

## Performance Optimizations

### 1. Parallel DA Fetching
```rust
let futures = providers.iter().map(|p| p.fetch_latest());
let results = futures::future::join_all(futures).await;
```

### 2. Connection Pooling
```rust
let pool = SqlitePool::new()
    .max_connections(100)
    .min_connections(10)
    .connection_timeout(Duration::from_secs(5))
    .build()?;
```

### 3. Prepared Statement Caching
```rust
let mut stmt_cache = LruCache::new(100);
if let Some(stmt) = stmt_cache.get(sql) {
    stmt.execute(params)?;
} else {
    let stmt = conn.prepare(sql)?;
    stmt_cache.put(sql.to_string(), stmt);
}
```

### 4. Read Replicas Load Balancing
```rust
let replicas = vec![replica1, replica2, replica3];
let selected = replicas[rand::random::<usize>() % replicas.len()];
selected.query(sql).await
```

## Security Considerations

### 1. Signature Verification
```rust
// All messages must be signed by the expected sequencer
// Signature verification happens before any data is applied
fn verify_message(&self, message: &SignedMessage) -> Result<()> {
    // Verify message_hash matches payload
    let computed_hash = keccak256(&message.payload);
    if computed_hash != message.message_hash {
        return Err(anyhow!("Payload hash mismatch"));
    }

    // Verify signature recovers to expected sequencer
    let recovered = recover_signer(&message)?;
    if recovered != self.expected_sequencer {
        return Err(anyhow!("Invalid sequencer signature"));
    }

    Ok(())
}
```

### 2. Read-Only Query Enforcement
```rust
// Replica serves read-only queries - no writes allowed through API
pub fn validate_query(sql: &str) -> Result<()> {
    let normalized = sql.trim().to_uppercase();
    if !normalized.starts_with("SELECT") {
        return Err(Error::ReadOnlyMode);
    }
    Ok(())
}
```

### 3. Rate Limiting
```rust
use tower::limit::RateLimitLayer;

let rate_limit = RateLimitLayer::new(100, Duration::from_secs(1));
let app = Router::new()
    .route("/query", post(query_handler))
    .layer(rate_limit);
```

### 4. Changeset Validation
```rust
// Changesets are applied atomically with conflict detection
fn apply_changeset(&self, data: &[u8]) -> Result<()> {
    let changeset = rusqlite::session::Changeset::new(data)?;

    // Apply with strict conflict handling - abort on any conflict
    changeset.apply(&self.conn, None::<fn(&str) -> bool>, |conflict| {
        error!("Changeset conflict: {:?}", conflict);
        rusqlite::session::ConflictAction::Abort
    })?;

    Ok(())
}
```

## Resource Requirements

### Read Replica
- **CPU**: 2+ cores
- **Memory**: 2GB minimum, 4GB recommended
- **Disk**: 50GB+ SSD (depends on database size)
- **Network**: 100Mbps minimum

### Validator
- **CPU**: 4+ cores (TEE-enabled for Confidential Space)
- **Memory**: 8GB minimum, 16GB recommended
- **Disk**: 200GB+ SSD
- **Network**: 1Gbps recommended
- **TEE**: GCP Confidential Space (AMD SEV-SNP)

## Monitoring Metrics

Key metrics exposed via Prometheus:

```rust
// src/metrics.rs
use prometheus::{IntCounter, IntGauge, Histogram};

lazy_static! {
    pub static ref SYNC_LAG: IntGauge = IntGauge::new(
        "synddb_sync_lag_sequences",
        "Number of sequences behind the latest"
    ).unwrap();

    pub static ref MESSAGES_APPLIED: IntCounter = IntCounter::new(
        "synddb_messages_applied_total",
        "Total messages applied"
    ).unwrap();

    pub static ref CHANGESETS_APPLIED: IntCounter = IntCounter::new(
        "synddb_changesets_applied_total",
        "Total changesets applied"
    ).unwrap();

    pub static ref SNAPSHOTS_APPLIED: IntCounter = IntCounter::new(
        "synddb_snapshots_applied_total",
        "Total snapshots restored"
    ).unwrap();

    pub static ref SIGNATURE_FAILURES: IntCounter = IntCounter::new(
        "synddb_signature_verification_failures_total",
        "Failed signature verifications"
    ).unwrap();

    pub static ref QUERY_LATENCY: Histogram = Histogram::with_opts(
        HistogramOpts::new("synddb_query_latency_seconds", "Query latency")
    ).unwrap();

    // Validator-only metrics
    pub static ref WITHDRAWALS_PROCESSED: IntCounter = IntCounter::new(
        "synddb_withdrawals_processed_total",
        "Withdrawals posted to L1"
    ).unwrap();
}
```

Key metrics:
- `synddb_sync_lag_sequences` - How many sequences behind the replica is
- `synddb_messages_applied_total` - Total messages processed
- `synddb_changesets_applied_total` - Total changesets applied
- `synddb_snapshots_applied_total` - Total snapshots restored
- `synddb_signature_verification_failures_total` - Failed signature verifications
- `synddb_query_latency_seconds` - Query response time histogram
- `synddb_withdrawals_processed_total` - Withdrawals posted to L1 (validator only)
