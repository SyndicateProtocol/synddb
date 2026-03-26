# SyndDB

### Build Any App. Use SQLite. Get Blockchain Verifiability.

---

## Our Goal

Providing the best developer experience for products to move onchain.

---

## What's Not Working

- Our biggest competitor? **Web2 infrastructure**
- Not losing to RaaS providers or L2s
- Losing to projects **never going onchain at all**

---

## The Reality

Most valuable crypto apps (perp DEXs, prediction markets, stablecoins) start as:

**Offchain databases + lightweight onchain components**

---

## The Problem

Asking developers to rewrite everything as smart contracts is a losing battle.

- Less scalable technology
- Far more moving parts
- EVM/SVM can't handle CEX-scale orderbooks

---

## Our Insight

Don't force them to change.

**Let them bring their databases. We wire them up to the chain.**

---

## Introducing SyndDB

Write apps in **any language**. Use **SQLite**. Get **blockchain verifiability**.

---

## Why SQLite?

- **Billions** of deployments worldwide
- Available in every programming language
- Deterministic execution
- Sub-millisecond writes
- Trivial to run inside a TEE
- Massive LLM training data

---

## The Developer Experience

```python
import sqlite3
from synddb import SyndDB

db = sqlite3.connect('app.db')
synddb = SyndDB.attach('app.db', 'https://sequencer:8433')

# Just write SQL. We handle the rest.
db.execute("INSERT INTO trades VALUES (?, ?)", (123, 'BUY'))
```

---

## How It Works

1. **Write apps** in any language with SQLite
2. **Import SyndDB client** (2-3 lines of code)
3. **Run in a TEE** (Docker container)
4. **We capture everything** and publish to DA layers

---

## Architecture Overview

```
Application (Any Language)
         ↓
    SQLite + SyndDB Client (TEE #1)
         ↓
    Sequencer Service (TEE #2)
         ↓
    DA Layers (Celestia, GCS, IPFS)
         ↓
    Validators (TEE) → Bridge Contract
```

---

## Component: Client Library

- Embeds in your application process
- Captures SQL operations via Session Extension
- Sends changesets to sequencer via HTTP
- Includes TEE attestation tokens

**Zero code changes** to your business logic.

---

## Component: Sequencer

- Runs in a **separate TEE** (key isolation)
- Receives changesets from client libraries
- Batches and compresses (zstd)
- Publishes to multiple DA layers
- Monitors blockchain for inbound messages

---

## Component: Validators

- Sync from DA layers
- Replay SQL operations
- Verify invariants (balances, limits)
- Sign for bridge settlement
- Anyone can run a read replica

---

## Component: Bridge Contract

- Processes validator-signed messages
- Handles deposits/withdrawals
- Modular validation (pre/post hooks)
- Inspired by Safe Guards, Hyperlane ISMs

---

## TEE Security Model

```
┌─────────────────────────────────────────────┐
│         GCP Confidential Space VM           │
│  ┌─────────────────┐  ┌─────────────────┐  │
│  │   Application   │  │    Sequencer    │  │
│  │   + Client      │  │   (signing keys)│  │
│  │   Container     │  │    Container    │  │
│  └────────┬────────┘  └────────┬────────┘  │
│           └──────────┬─────────┘           │
│              Shared SQLite DB              │
│  Hardware Root of Trust (AMD SEV-SNP)      │
└─────────────────────────────────────────────┘
```

---

## Verifiability Model

SQL operations **are** the audit trail.

- Application writes everything to SQLite
- Sequencer publishes to DA layers
- Validators verify SQL operations, not code
- TEE attestations prove correct execution

---

## What Validators Check

**Default (automatic):**
- SQL syntax and semantics
- State transition validity
- Balance consistency
- Message passing rules

**Optional extensions:**
- Price feed verification
- Rate limiting
- Custom business rules

---

## Message Passing

Applications define tables. We handle the bridge.

```sql
CREATE TABLE outbound_withdrawals (
    id INTEGER PRIMARY KEY,
    recipient TEXT NOT NULL,
    amount INTEGER NOT NULL,
    status TEXT DEFAULT 'pending'
);
```

Sequencer detects changes. Validators sign. Bridge executes.

---

## Trade-offs

For this performance, applications accept:

1. **Centralized app instance** (same as rollup sequencers)
2. **Non-EVM execution** (SQL instead of Solidity)
3. **Asset location choices** (native vs bridged)

---

## The Comparison

| | Traditional Onchain | Traditional Offchain | SyndDB |
|---|---|---|---|
| Latency | ~seconds | sub-ms | sub-ms |
| Throughput | 10-50 ops/s | unlimited | unlimited |
| Verifiable | yes | no | yes |
| Any language | no | yes | yes |

---

## Ideal Use Cases

- **Perp DEXs** - orderbook matching at scale
- **Prediction markets** - high-frequency updates
- **Stablecoin payments** - instant settlements
- **Gaming** - real-time state
- **NFT marketplaces** - metadata and trading

---

## Migration Path

Already using SQLite? Even easier.

1. Import SyndDB client library
2. Add message tables (if needed)
3. Deploy sequencer in separate TEE
4. **No other code changes required**

---

## Why This Matters

- **10X** the quantity and quality of crypto apps
- **10%** of the development effort
- Meet projects **where they're at**
- Build for what people need **now**

---

## The Vibe

Works for a hackathon participant.

Works for the next Hyperliquid.

Easier to use. More differentiated. More pricing power.

---

## Summary

- Write apps in **any language**
- Use **SQLite** (battle-tested, universal)
- Get **blockchain verifiability** for free
- **Sub-millisecond latency**, unlimited throughput
- Perfect for **orderbooks** and high-value apps

---

## Questions?

Reach out if you have use cases for:
- High throughput applications
- Orderbooks (perp DEXs, prediction markets)
- Payments and stablecoins
- Gaming, social, NFT marketplaces
