# SyndDB: High-Performance Blockchain Database

## Overview

SyndDB enables developers to build high-performance blockchain applications using **any programming language** with SQLite bindings. Instead of learning a new framework, developers write applications in their preferred language (Python, JavaScript, Go, Rust, etc.) that persist data to SQLite, while SyndDB infrastructure automatically captures and publishes the SQL operations for verification and replication.

The key insight is that **SQL operations themselves become the verifiable audit trail**. The application runs inside a TEE with attestations proving it's running the correct code, while validators (also in TEEs) verify the SQL operations and their effects. Rather than requiring validators to re-execute complex business logic and external API calls, they focus on auditing database operations, making verification practical without sacrificing application flexibility.

The architecture is simple:

1. **Write applications in any language** that use SQLite for persistence
2. **Import the SyndDB client library** to automatically capture changesets and snapshots
3. **Run a sequencer service** (in a separate TEE) that receives and publishes SQL operations to DA layers
4. **Enable permissionless read replicas** that sync the SQL operations to serve queries
5. **Support message passing** through application-defined tables that trigger cross-chain operations

**Deployment**: SyndDB is built on **Google Cloud Confidential Space** as its primary TEE platform, providing hardware-backed attestation and Workload Identity Federation. One-click deployment will be available through the **GCP Marketplace** for production-ready infrastructure.

Developers integrate SyndDB by importing a lightweight client library (available for Rust, Python, Node.js, and any language via C FFI). The library automatically captures SQL operations and sends them to the sequencer service, which handles all DA layer publishing. This design delivers ultra-low latency (<1ms local writes) and high throughput while maintaining verifiability at the SQL level. Applications can use any programming language, frameworks, libraries, or external services - as long as the results are persisted to SQLite, the system captures everything needed for verification.

## Why SyndDB?

**Use any language, get blockchain verifiability for free.** SyndDB lets you build high-performance applications in Python, JavaScript, Go, Rust, or any language with SQLite - no framework to learn, no code changes required. Get offchain performance (sub-millisecond latency, unlimited throughput) with onchain transparency (all state verifiable through SQL operations). Your application runs in a TEE for accountability, while validators provide additional guardrails before settlement. Perfect for applications where ultra-low latency matters more than full decentralization: orderbooks, gaming, social feeds, real-time analytics.

### Why SQLite?

SQLite is the ideal foundation for verifiable blockchain applications:

- **Trivial TEE Colocation**: Runs in-process with your application - no separate database server, simple to deploy together inside a single TEE
- **Deterministic Execution**: Same SQL operations always produce the same results, making verification straightforward
- **High Performance**: Zero-copy reads, sub-millisecond writes, millions of operations per second
- **Extensible**: Add custom business logic via user-defined functions and triggers without changing application code
- **Universal Support**: Available in virtually every programming language (Python, JavaScript, Go, Rust, Java, C++, etc.)
- **Proven Stability**: Battle-tested database engine with billions of deployments worldwide

## Key Benefits

1. **Use any language and framework** - Write applications in Python, JavaScript, Go, Rust, or any language with SQLite bindings. Zero code changes required to adopt SyndDB if you already use SQLite.
2. **Incredibly fast and low latency** - Sub-millisecond local writes with high throughput
3. **Flexible asset management** - Assets can either:
   - Live natively on the system for maximum performance, or
   - Remain on the settlement chain with actions triggered via message passing
   - Bridge only when needed (bridging is optional, not required)
4. **Built-in indexing** - Few to no indexing requirements (indexing is built into the relational database)

## Trade-offs

For this performance, applications must accept:

1. **Centralized application instance** (same trust model as rollup sequencers, but with better performance)
   - Liveness depends on the application instance staying online
   - Fallback instances may restart from last published state (potential data loss between publications)
   - Validators remain decentralized for security
2. **Non-EVM execution framework** - Uses SQL instead of Solidity/EVM
3. **Asset location flexibility** comes with different trade-offs:
   - Assets on settlement layer: Maximum security but requires message passing for actions
   - Assets native on SyndDB: Maximum performance but relies on application and validator security model
   - Hybrid approach: Bridge assets as needed for specific operations (adds operational complexity)

## Architecture Overview

SyndDB makes any SQLite application blockchain-verifiable by automatically capturing SQL operations (as changesets) and publishing them with periodic snapshots.

### Core Components

1. **Application - Any Language**
   - Written in any language (Python, Node.js, Go, Rust, Java, etc.)
   - Uses SQLite for persistence (via language-specific SQLite bindings)
   - Imports SyndDB client library (Rust/Python/Node.js or via C FFI)
   - Runs inside a TEE for attestation and accountability
   - Can use any libraries, frameworks, or external APIs
   - All state changes must be persisted to SQLite

2. **SyndDB Client Library**
   - Embedded in the application process (same TEE as application)
   - Attaches to the SQLite database via Session Extension (official SQLite API for capturing logical changes deterministically)
   - Captures SQL operations as changesets (INSERT/UPDATE/DELETE with values)
   - Creates periodic snapshots for recovery points
   - Detects schema changes and triggers immediate snapshots
   - Sends changesets and snapshots to sequencer service via HTTP
   - Includes TEE attestation tokens proving application workload identity
   - Available for Rust, Python, Node.js, and any language via C FFI

3. **Sequencer Service** (runs in separate TEE on Google Cloud Confidential Space)
   - Receives changesets and snapshots from application client libraries via HTTP
   - Verifies TEE attestation via Confidential Space OIDC tokens and Workload Identity Federation
   - Batches and publishes to DA layers (Celestia, EigenDA) and storage layers (IPFS, Arweave)
   - Monitors blockchain for inbound messages and delivers them to applications
   - Holds signing keys for DA layer publishing (isolated from application)
   - **Security Note**: Runs in a separate TEE from the application to prevent key extraction, as deploying multiple containers in a single TEE is complex and creates attack surface
   - **Deployment**: Will be available as a one-click deployment through **GCP Marketplace** for production-ready infrastructure

4. **Read Replicas**
   - Anyone can run a read replica permissionlessly
   - Sync SQL operations (changesets and snapshots) from DA layers
   - Replay SQL operations to maintain consistent database state
   - Serve queries with full SQL capabilities

5. **Validators**
   - Read replicas with validation logic that runs in TEEs
   - Verify SQL operations (changesets) before signing for settlement
   - Default checks: Operation validity, state consistency, balance invariants
   - Optional checks: External API verification, custom business rules
   - Process cross-chain messages from message tables

6. **Bridge (Message Passing)**
   - Smart contract for processing cross-chain messages
   - Sequencer service monitors message tables with schemas tied to contract ABI
   - Processes outbound messages (withdrawals, cross-chain calls)
   - Delivers inbound messages to application (deposits, cross-chain responses)

### Data Flow

```
Application (Any Language) → SQLite + SyndDB Client → Sequencer Service → DA Layers → Validators (TEE) → Blockchain → Bridge.sol
    in TEE (Application)             in TEE (Client)    in separate TEE                     ↓
                                            ↓                                          Bridge Contract
                                    Message Tables
                                    (HTTP to Sequencer)
```

**Application Path**:
1. App writes to SQLite
2. SyndDB client library captures changesets/snapshots (same process, same TEE)
3. Client sends to sequencer service via HTTP with TEE attestation
4. Sequencer publishes to DA layers

**Validator Path**: Validators read from DA → Verify SQL → Post to blockchain

**Message Passing**: Validators detect messages in SQL → Process via Bridge.sol

**TEE Isolation**: The application (with client library) runs in one TEE, while the sequencer service runs in a separate TEE. This architectural separation prevents the application from accessing the sequencer's signing keys, which is critical for security. Deploying multiple containers in a single TEE is complex and increases attack surface, so the separation provides defense in depth.

Validators can subscribe to the sequencer (with TEE attestation) for lower latency instead of waiting for DA publication. The application never touches the blockchain directly.

### Application as Source of Truth

The application runs in a TEE with hardware attestations proving it's running the correct code. This provides accountability while maintaining performance.

Key properties:

- **TEE Accountability**: Attestations prove the application is running unmodified code
- **Validator Guardrails**: Validators enforce limits (withdrawal caps, rate limits) before settlement
- **Flexible Implementation**: Use any language, external APIs, or libraries - only SQL output matters
- **Performance Optimized**: Can prune historical data while providing snapshots for bootstrapping

This trades full decentralization for extreme performance, suitable for applications where ultra-low latency matters more than fully decentralized execution.

## Verifiability Model: SQL Operations as the Audit Trail

Unlike traditional rollups that require full re-execution of all logic, SyndDB uses SQL operations as the verifiable audit trail. This fundamental shift enables practical verifiability without sacrificing application flexibility.

### Four Pillars of SyndDB Verifiability

1. **Application Writes Everything to SQL**: The application must persist all data that could affect state transitions to SQLite, including:
   - Application state changes
   - Logs of external API calls and their results
   - User inputs and their effects
   - Message passing operations via special tables

2. **Client Library and Sequencer Publish to DA Layers**: The SyndDB client library (embedded in the application) captures SQL operations and sends them to the sequencer service, which publishes to censorship-resistant DA layers:
   - SQL operations are captured as changesets (INSERT/UPDATE/DELETE with values)
   - Sequence numbers maintaining strict ordering
   - Periodic snapshots for bootstrapping and recovery
   - Immediate snapshots on schema changes (DDL operations like ALTER TABLE automatically trigger a full snapshot, ensuring validators can always reconstruct the complete database schema and state)
   - This ensures data is widely available and reduces equivocation risk

3. **Validators Verify SQL Operations, Not Code**: Validators read from DA layers and check:
   - SQL syntax and semantic correctness (default validation)
   - State transitions make sense (default validation)
   - Balances remain consistent (default validation)
   - Message passing operations follow rules (default validation)
   - Custom business logic checks (optional extensions)
   - Best-effort re-derivation of external data (optional extensions)

4. **Re-execution is Optional**: By default, validators don't need to:
   - Re-run the original application (which could be in any language)
   - Perfectly reproduce all external API calls
   - Re-compute complex algorithms exactly
   - Match non-deterministic operations

   However, validators can be extended to perform more thorough re-execution if desired - this is purely optional and application-specific.

### Why This Works

TEE attestations ensure the application runs the correct code. Validators verify the SQL operations (captured as changesets), not the implementation. This is like database replication via changesets - apply the same principle to blockchain, with TEEs preventing equivocation and validators adding checks before settlement.

### Example: High-Performance Orderbook

**Traditional Onchain Approach** (validated but slow):

- Every order placement/cancellation is a blockchain transaction
- Matching logic runs in EVM
- Throughput limited to ~10-50 orders/second
- Transparent and verifiable, but impractical for high-frequency trading

**Traditional Offchain Approach** (fast but not validated):

- Orders processed in centralized database
- No transparency into matching logic
- High throughput, but trust the exchange operator

**SyndDB Approach** (best of both worlds):

```python
# High-performance matching engine in any language
def match_orders(buy_order, sell_order):
    # Complex matching logic with sub-millisecond latency
    # Use any libraries, optimizations, algorithms

    # Just persist the results to SQLite
    cursor.execute("""
        INSERT INTO trades (buy_order_id, sell_order_id, price, quantity)
        VALUES (?, ?, ?, ?)
    """, (buy_order.id, sell_order.id, match_price, quantity))

    cursor.execute("""
        UPDATE orders SET status = 'FILLED'
        WHERE id IN (?, ?)
    """, (buy_order.id, sell_order.id))

    db.commit()
```

**Benefits**:

- Offchain performance: Sub-millisecond order matching, unlimited throughput
- Onchain transparency: All trades verifiable through published SQL operations
- Validator checks: Can add business logic like "no self-trading" or "price must be within spread" without re-implementing the matching engine
- Flexibility: Upgrade matching logic without changing validators

## Smart Contracts and Message Passing

### Publishing Model: Application → DA → Validators → Blockchain

The application never touches the blockchain - the sequencer publishes to DA layers:

0. **Application → SQLite**: Application writes all state changes to SQLite database
1. **Client Library → Sequencer**: SyndDB client library (running in application process) captures changesets/snapshots and sends them to the sequencer service via HTTP with TEE attestation
2. **Sequencer → DA**: The sequencer service (running in a separate TEE) publishes SQL operations (changesets/snapshots) to DA/storage layers (Celestia, EigenDA, IPFS, Arweave) with sequencer TEE signatures
3. **DA → Validators**: Validators sync from censorship-resistant DA layers
4. **Validators → Blockchain**: Post verified state transitions to settlement contract. Messages in the bridge tables are processed via Bridge.sol.

This keeps the application isolated from blockchain infrastructure while enabling multiple DA sources for resilience. No custom bridge code needed - just define tables that match your message schema.

### Validator Settlement Contract

Validators (not the application) interact with the blockchain:

```solidity
// Reference to DA layer data for transparency
// Changesets contain logical database changes (INSERT/UPDATE/DELETE operations)
function submitChangeset(string calldata daCid, bytes32 dataHash, uint256 sequenceNumber)

// Snapshots are full database states for recovery/bootstrapping
function submitSnapshot(string calldata daCid, bytes32 dataHash, uint256 sequenceNumber)
```

The settlement contract only accepts state updates signed by a threshold of validators running in TEEs.

### Message Passing Contract

Message passing operations are triggered when the application writes to message tables. The contract ABI is tied to the table schema, allowing flexible message types:

```solidity
// Generic message processing from outbound_messages table
function processMessage(
    uint256 messageId,
    bytes calldata messageData,  // Decoded from table schema
    bytes[] validatorSignatures
)

// Receives inbound messages and sequencer delivers them to application
function sendMessage(
    string calldata targetAccountId,
    bytes calldata messageData
)
```

Common message types include:

- Asset withdrawals/deposits (bridge operations)
- Cross-chain function calls
- Oracle data requests/responses
- Governance proposals

### Message Tables

Applications define message tables with schemas that map to contract ABIs:

```sql
-- Example: Asset withdrawal messages
CREATE TABLE outbound_withdrawals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL,
    token_address TEXT NOT NULL,
    amount INTEGER NOT NULL,
    destination_address TEXT NOT NULL,
    status TEXT DEFAULT 'pending',
    created_at INTEGER DEFAULT (unixepoch())
);

-- Example: Inbound deposit messages
CREATE TABLE inbound_deposits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_tx_hash TEXT UNIQUE NOT NULL,
    account_id TEXT NOT NULL,
    token_address TEXT NOT NULL,
    amount INTEGER NOT NULL,
    block_number INTEGER NOT NULL,
    created_at INTEGER DEFAULT (unixepoch())
);

-- Example: Generic cross-chain calls
CREATE TABLE outbound_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    target_chain TEXT NOT NULL,
    target_contract TEXT NOT NULL,
    function_signature TEXT NOT NULL,
    parameters BLOB NOT NULL,
    status TEXT DEFAULT 'pending'
);
```

## Validators and Settlement

### Default Validator Implementation

SyndDB provides a default validator implementation that:

1. **Replays SQL Operations**: Applies changesets to rebuild state deterministically
2. **Verifies Bridge Claims**: Confirms all withdrawal/deposit amounts match bridge table entries
3. **Validates Basic Invariants**: Ensures balances never go negative, totals sum correctly
4. **Signs Valid States**: Approves states that pass all basic checks

### Extending the Default Validator

The default validator is designed to be extended with custom logic:

```rust
// Start with the default validator
use synddb_validator::DefaultValidator;

impl CustomValidator {
    fn validate(&self, changesets: &[Changeset]) -> Result<()> {
        // Run default validation first
        self.default.validate(changesets)?;

        // Add external API checks
        if self.config.check_oracles {
            self.verify_price_feeds(changesets)?;
        }

        // Add guardrails on amounts
        for changeset in changesets {
            if let Some(withdrawal) = parse_withdrawal(changeset) {
                // Reject anomalous movements
                if withdrawal.amount > self.config.max_withdrawal {
                    return Err("Withdrawal exceeds maximum");
                }
                if withdrawal.amount > self.get_historical_average() * 10 {
                    return Err("Withdrawal 10x above average");
                }
            }
        }

        // Add any other custom logic
        self.check_kyc_requirements(changesets)?;
        self.verify_rate_limits(changesets)?;

        Ok(())
    }
}
```

### TEE Deployment (Required)

Validators **must** run in Trusted Execution Environments to ensure they are running unmodified code. **Google Cloud Confidential Space** is the primary supported TEE platform:

- **Google Cloud Confidential Space**: Production-grade TEE platform using Confidential VMs with AMD SEV or Intel TDX hardware isolation, Workload Identity Federation, and the Google Cloud Attestation service
- **Attestation**: TEE validators prove they're running unmodified code via Confidential Space OIDC attestation tokens
- **Protected Keys**: Settlement keys remain secure within the TEE boundary
- **Changeset Verification**: Hardware-backed guarantees prevent validator subversion
- **Message Processing**: TEE validators can hold bridge signing authority
- **One-Click Deployment**: Validators will be available through **GCP Marketplace** for streamlined production deployment

### Running the Default Validator

Deploy the default validator in a TEE:

```bash
# Run the validator (fetching from sequencer's HTTP API)
synddb-validator \
    --fetcher-type http \
    --sequencer-url http://sequencer:8433 \
    --sequencer-pubkey 8318535b...

# Or fetch from GCS
synddb-validator \
    --fetcher-type gcs \
    --gcs-bucket my-bucket \
    --sequencer-pubkey 8318535b...
```

The default validator will sync SQL operations (changesets and snapshots) from storage layers, apply them to rebuild state, and verify basic invariants before signing for settlement.

Applications can be in any language, while validators share the same implementation to ensure consistent verification across the network.

## Use Cases

SyndDB is designed for high-scale applications that require ultra-low latency and high throughput, including:

- Onchain order books for perp DEXs
- Gaming state and leaderboards
- Social applications and feeds
- NFT marketplaces and metadata
- Real-time analytics and dashboards

## Implementation Guide

### Building a SyndDB Application

1. **Import SyndDB Client Library**

   **Rust Example:**

   ```rust
   use rusqlite::Connection;
   use synddb_client::SyndDB;

   // Connection must have 'static lifetime
   let conn = Box::leak(Box::new(Connection::open("app.db")?));

   // Attach SyndDB client - automatically captures changesets
   let synddb = SyndDB::attach(conn, "https://sequencer.example.com:8433")?;

   // Use SQLite normally
   conn.execute("INSERT INTO trades VALUES (?1, ?2)", params![1, 100])?;

   // Changesets are automatically sent to sequencer every 1 second
   // Or force immediate publish for critical transactions:
   synddb.publish_changeset()?;
   ```

   **Python Example:**

   ```python
   import sqlite3
   from synddb import SyndDB
   from flask import Flask

   app = Flask(__name__)
   db = sqlite3.connect('app.db', check_same_thread=False)

   # Attach SyndDB client - automatically captures changesets
   synddb = SyndDB.attach('app.db', 'https://sequencer.example.com:8433')

   @app.route('/trade')
   def execute_trade():
       # Your business logic here
       cursor = db.cursor()
       cursor.execute("INSERT INTO trades ...")
       db.commit()
       # Changesets automatically sent to sequencer
   ```

   **Node.js Example:**

   ```javascript
   const Database = require("better-sqlite3");
   const { SyndDB } = require("synddb");
   const express = require("express");

   const app = express();
   const db = new Database("app.db");

   // Attach SyndDB client - automatically captures changesets
   const synddb = SyndDB.attach("app.db", "https://sequencer.example.com:8433");

   app.post("/trade", (req, res) => {
     // Your business logic here
     db.prepare("INSERT INTO trades ...").run();
     // Changesets automatically sent to sequencer
   });
   ```

2. **Create Bridge Tables (if needed)**

   ```sql
   -- Add these tables if you need bridge functionality
   CREATE TABLE bridge_withdrawals (...);
   CREATE TABLE bridge_deposits (...);
   ```

3. **Deploy Application and Sequencer (Separate TEEs)**

   ```yaml
   # docker-compose.yml
   services:
     # Application with SyndDB client library (TEE #1)
     app:
       image: your-app:latest
       environment:
         - SYNDDB_SEQUENCER_URL=https://sequencer:8433
       # Deploy in TEE (GCP Confidential Space, AWS Nitro, etc.)
       # Client library includes TEE attestation automatically

     # Sequencer service (TEE #2 - separate for key isolation)
     synddb-sequencer:
       image: syndicate/synddb-sequencer:latest
       ports:
         - "8433:8433"
       environment:
         - CHAIN_RPC=https://...
         - CELESTIA_NODE=https://...
         - IPFS_GATEWAY=https://...
       # Deploy in separate TEE to isolate signing keys from application
   ```

   **Security Note**: The application and sequencer run in **separate TEEs**. This is critical because:
   - The sequencer holds signing keys for storage layer publishing
   - Deploying multiple containers in a single TEE is complex and increases attack surface
   - TEE separation provides defense in depth - even if the application is compromised, signing keys remain isolated

4. **Run Read Replicas**

   ```bash
   # Anyone can run a read replica
   synddb-replica \
     --chain-rpc https://... \
     --start-block 12345 \
     --serve-port 8080
   ```

5. **Deploy in TEEs (Required for Production)**

   SyndDB uses **Google Cloud Confidential Space** as its primary TEE platform. There are two deployment options:

   **Option A: GCP Marketplace (Coming Soon)**

   One-click deployment of production-ready SyndDB infrastructure will be available via the **GCP Marketplace**:
   - Pre-configured Confidential Space VMs with SyndDB sequencer and validator images
   - Automatic attestation and Workload Identity Federation setup
   - Integrated monitoring and logging

   **Option B: Manual Deployment**

   ```bash
   # Application TEE (GCP Confidential Space)
   gcloud compute instances create app-tee \
     --confidential-compute \
     --image-project=confidential-space-images \
     --image-family=confidential-space \
     --container-image=your-app:latest

   # Sequencer TEE (separate instance for key isolation)
   gcloud compute instances create sequencer-tee \
     --confidential-compute \
     --image-project=confidential-space-images \
     --image-family=confidential-space \
     --container-image=syndicate/synddb-sequencer:latest
   ```

### Key Design Principles

- **Language Agnostic**: Use any programming language, framework, or runtime
- **Client Library Integration**: Import lightweight SyndDB client library (Rust/Python/Node.js or C FFI)
- **SQL as Truth**: All state changes must go through SQLite for capture
- **Automatic Capture**: Client library automatically captures changesets and snapshots
- **Automatic Publishing**: Sequencer service handles all storage layer and blockchain interaction
- **TEE Isolation**: Application and sequencer run in separate TEEs for key isolation
- **Consistent Validation**: Validators use the same implementation for verification
- **Permissionless Replication**: Anyone can sync and query the data
- **Optional Message Passing**: Add message tables only if you need cross-chain operations

### Migration from Existing Applications

Converting any existing SQLite application to SyndDB is straightforward:

1. Ensure all state changes go through SQLite (not just in-memory)
2. Add message tables if you need cross-chain message passing functionality
3. **Import SyndDB client library** - add 2-3 lines of code to attach to your existing connection:
   ```python
   # Python example
   from synddb import SyndDB
   synddb = SyndDB.attach('app.db', 'https://sequencer.example.com:8433')
   ```
4. Deploy sequencer service in a separate TEE
5. No other code changes required to your business logic

This approach makes SyndDB a drop-in solution for adding blockchain verifiability to applications written in any language.

## Wire Format: CBOR/COSE Binary Encoding

SyndDB uses CBOR (Concise Binary Object Representation) with COSE (CBOR Object Signing and Encryption) for efficient, authenticated message storage and transport.

### Overview

The wire format provides:
- **Up to 40% size reduction** compared to JSON+base64 encoding
- **Cryptographic authenticity** via COSE_Sign1 signatures
- **Content addressing** via SHA-256 hashes for cross-system references
- **Transport agnosticism** - same format works across GCS, Arweave, etc.

### Message Structure: COSE_Sign1

Individual messages are wrapped in COSE_Sign1 structures (RFC 9052). The structure contains:

```
COSE_Sign1 = [
    protected: bstr,     # CBOR-encoded protected header
    unprotected: {},     # Unprotected header (signer public key)
    payload: bstr,       # zstd-compressed payload
    signature: bstr      # 64-byte secp256k1 signature (r || s)
]
```

#### Protected Header

The protected header is CBOR-encoded and covered by the signature. It contains:

| Field | COSE Label | Type | Description |
|-------|------------|------|-------------|
| Algorithm | 1 | int | ES256K (-47) for secp256k1 |
| Sequence | -65537 | uint | Monotonic sequence number |
| Timestamp | -65538 | uint | Unix timestamp |
| Message Type | -65539 | uint | 0=Changeset, 1=Withdrawal, 2=Snapshot |

The custom labels (-65537 to -65539) are in the IANA private use range.

#### Unprotected Header

| Field | Label | Type | Description |
|-------|-------|------|-------------|
| Signer | "signer" | bstr | 64-byte uncompressed secp256k1 public key (without 0x04 prefix) |

#### Signature Format

The signature is 64 bytes (r || s) without the recovery byte `v`. During verification, the signature is verified directly against the signer's public key using ECDSA verification (no address recovery needed).

The signature covers the COSE `Sig_structure`:
```
Sig_structure = [
    context: "Signature1",
    body_protected: protected_header_bytes,
    external_aad: bstr_empty,
    payload: payload_bytes
]
```

### Batch Structure: CborBatch

Multiple messages are grouped into batches for efficient storage:

```
CborBatch = {
    "v":    uint,      # Format version (currently 1)
    "s":    uint,      # Start sequence (inclusive)
    "e":    uint,      # End sequence (inclusive)
    "t":    uint,      # Creation timestamp
    "h":    bstr,      # SHA-256 content hash (32 bytes)
    "m":    [bstr],    # Array of COSE_Sign1 message bytes
    "sig":  bstr,      # 64-byte batch signature (r || s)
    "pub":  bstr       # 64-byte signer public key
}
```

#### Content Hash

The content hash is SHA-256 over all message bytes concatenated in order:
```
content_hash = SHA256(message[0].bytes || message[1].bytes || ... || message[n].bytes)
```

This enables content-addressed lookup across different storage systems.

#### Batch Signature

The batch signature covers:
```
signing_payload = keccak256(start_sequence_be || end_sequence_be || content_hash)
```

Where `_be` indicates big-endian 8-byte encoding.

### Storage Format

Batches are stored as CBOR serialized data compressed with zstd (level 3).

**File naming convention:**
```
{prefix}/batches/{start:012}_{end:012}.cbor.zst
```

Examples:
- `sequencer/batches/000000000001_000000000050.cbor.zst` (messages 1-50)
- `sequencer/batches/000000000051_000000000100.cbor.zst` (messages 51-100)

The 12-digit zero-padding supports approximately 1 trillion sequences while maintaining lexicographic sortability. If you were to send messages every second, this is approximately 300 centuries, or roughly the amount of time from the caveman era to the modern day. 

**Content type:** `application/cbor+zstd`

### Transport Layer Architecture

The transport layer is abstracted from the batch format:

```
┌─────────────────────────────────────────────────────────────┐
│                        CborBatch                            │
│  (format-agnostic: same structure regardless of transport)  │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                   TransportPublisher                        │
├─────────────────────┬───────────────────────────────────────┤
│   GcsTransport      │   ArweaveTransport (future)          │
│   - Raw CBOR+zstd   │   - ANS-104 DataItem wrapper         │
│   - gs:// URIs      │   - Discovery tags                   │
│                     │   - Content hash cross-reference     │
└─────────────────────┴───────────────────────────────────────┘
```

**Current:** GCS stores raw CBOR+zstd bytes directly.

**Future (Arweave):** CBOR batch wrapped in ANS-104 DataItem with tags:
- `App-Name`: Application identifier
- `Schema-Version`: Format version
- `Start-Sequence`, `End-Sequence`: Sequence range
- `Content-SHA256`: Content hash for cross-system lookup

The `content_hash` field enables content-addressed lookup regardless of transport-specific addressing (Arweave TX IDs differ even for identical content).

### Verification Flow

1. **Batch verification:**
   - Recompute content hash from messages
   - Verify batch signature against signer public key
   - Confirm signer public key matches expected sequencer

2. **Message verification:**
   - Parse COSE_Sign1 structure
   - Verify signature covers correct `Sig_structure`
   - Verify signature is valid for the claimed public key (direct ECDSA verification)
   - Confirm signer public key matches expected sequencer
   - Validate protected header fields match outer message fields

3. **Field consistency:**
   - Outer `sequence` must equal protected header sequence
   - Outer `timestamp` must equal protected header timestamp
   - This prevents field substitution attacks

## Terminology Glossary

### Platform and Deployment

- **Google Cloud Confidential Space** - SyndDB's primary TEE platform, using Confidential VMs with AMD SEV or Intel TDX hardware isolation, Workload Identity Federation, and the Google Cloud Attestation service for secure application and sequencer deployment
- **GCP Marketplace** - One-click deployment option (coming soon) for production-ready SyndDB infrastructure, including pre-configured Confidential Space VMs with SyndDB sequencer and validator images

### Core Architecture Terms

- **SyndDB** - Infrastructure that enables applications (any language) using SQLite to publish SQL operations to blockchain
- **SyndDB Client Library** - Lightweight library (Rust/Python/Node.js or C FFI) that embeds in applications, captures changesets/snapshots via SQLite Session Extension, and sends them to the sequencer service
- **Sequencer Service** - Server process (running in a separate TEE) that receives changesets/snapshots from client libraries and publishes them to storage layers and blockchain
- **SQL Operations** - Database modifications (INSERT/UPDATE/DELETE) that form the verifiable audit trail. Captured as changesets for efficient replication.
- **Changesets** - The technical mechanism for capturing SQL operations: deterministic logical database changes via SQLite Session Extension, more compact and auditable than physical page changes
- **Snapshots** - Complete database state at a point in time, published periodically for recovery/bootstrapping and immediately on schema changes

### Node Types

- **Application** - Your application (any language) running inside a TEE with SQLite and SyndDB client library, sending SQL operations to sequencer service
- **Sequencer** - Service running in a separate TEE that receives operations from applications and publishes to storage layers (holds signing keys, isolated from application)
- **Read Replica** - Any node that syncs published SQL operations to serve queries (anyone can run permissionlessly)
- **Validator** - Read replica with additional validation logic that runs in a TEE and verifies SQL operations before signing for settlement

### State Management Terms

- **State Diff** - Batched SQL operations (captured as changesets) representing incremental database changes, published to storage layers
- **State Snapshot** - Complete SQLite database file at a specific version, published to storage layers for bootstrapping and recovery (also published immediately on schema changes)
- **State Update** - Generic term for either a changeset or snapshot. The cryptographic hash of a state update uniquely identifies that version of the database. This term replaces "state root" used in Merkle-based blockchains, since SyndDB uses changesets/snapshots rather than Merkle trees.
- **State Commitment** - Signed message published by the sequencer containing a state update hash, system status (Healthy/Degraded/Halted), and metadata. This TEE-signed attestation allows validators to verify the sequencer's view of the system state. Similar to how rollups publish "state commitments" that include state roots plus metadata, but adapted for SyndDB's non-Merkle architecture.
- **Sequence Number** - Monotonically increasing counter ensuring strict ordering of SQL operations
- **Settlement** - Process where validators publish verified state to blockchain after reading from storage layers

### Message Passing Components

- **Message Tables** - Special SQLite tables for cross-chain operations (e.g., `outbound_messages`, `inbound_messages`) monitored by validators
- **Bridge.sol** - Smart contract that processes messages from message tables, with ABI tied to table schema
- **Message Passing** - Automatic detection and processing of cross-chain messages via application-defined table schemas that map to smart contract ABIs
