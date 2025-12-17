# WAL vs Changeset Replication: Tradeoffs for SyndDB

This document evaluates two approaches to SQLite replication and explains why SyndDB uses changeset-based replication via the SQLite Session Extension.

## Background

SyndDB needs to capture database changes and publish them for validator verification. Two primary approaches exist:

1. **WAL-based replication** - Capture Write-Ahead Log frames (physical page changes)
2. **Changeset-based replication** - Capture logical changes via Session Extension

Tools like [Litestream](https://litestream.io/) use WAL-based replication for disaster recovery. This document evaluates whether that approach would work for SyndDB.

## Comparison Summary

| Aspect | WAL-Based | Changeset-Based |
|--------|-----------|-----------------|
| **Capture level** | Physical (pages) | Logical (rows) |
| **Integration** | External daemon | In-process library |
| **What's captured** | All page writes | INSERT/UPDATE/DELETE |
| **Auditability** | Opaque bytes | Inspectable operations |
| **Payload size** | Larger (full pages) | Smaller (changed values) |
| **Determinism** | Architecture-dependent | Architecture-independent |
| **Complexity** | Monitor files | Session lifecycle |

## WAL-Based Replication

### How It Works

SQLite's Write-Ahead Log (WAL) mode writes changes to a `-wal` file before checkpointing them to the main database. WAL replication:

1. Monitors the WAL file for new frames
2. Copies frames to replica storage
3. Periodically captures full database snapshots
4. Replays WAL frames on top of snapshots for recovery

### Advantages

| Benefit | Description |
|---------|-------------|
| **Zero application integration** | Just monitor file changes, no code modifications |
| **Complete capture** | Gets everything: data, schema, pragmas, vacuum |
| **No runtime overhead** | SQLite writes WAL anyway; just copy the frames |
| **Stable format** | WAL format is well-documented and rarely changes |
| **Point-in-time recovery** | Can restore to any WAL frame |

### Disadvantages

| Drawback | Description |
|----------|-------------|
| **Physical, not logical** | Contains page bytes, not "INSERT INTO users" |
| **Architecture-dependent** | Page layout may differ across platforms |
| **Larger payloads** | Full 4KB pages vs just changed column values |
| **Opaque to validators** | Can't easily inspect "what operation occurred" |
| **Checkpoint coordination** | Must prevent SQLite from checkpointing before capture |
| **Schema changes implicit** | Must parse page content to detect DDL |

### Litestream Specifics

[Litestream](https://litestream.io/) is a well-maintained WAL replication tool that:

- Runs as an external daemon or sidecar
- Supports S3, GCS, Azure, SFTP, and other backends
- Uses "generations" (WAL eras) with frame indices
- Provides point-in-time recovery within retention window

**What Litestream lacks for SyndDB:**

- Global sequence numbers (has generation + frame index, not monotonic sequence)
- Cryptographic signing (no COSE_Sign1 or similar)
- TEE attestation integration
- Logical change visibility for validators

## Changeset-Based Replication

### How It Works

The [SQLite Session Extension](https://www.sqlite.org/sessionintro.html) hooks into SQLite's internal change tracking:

1. Attach a session to a database connection
2. Session records logical changes (INSERT/UPDATE/DELETE with values)
3. Generate changeset blob on commit
4. Apply changesets to replicas for deterministic reconstruction

### Advantages

| Benefit | Description |
|---------|-------------|
| **Logical operations** | "UPDATE users SET balance=100 WHERE id=5" |
| **Auditable** | Validators can inspect exactly what changed |
| **Compact** | Only changed columns, not full pages |
| **Deterministic** | Same changesets produce same results everywhere |
| **Schema-aware** | Session extension knows about DDL changes |
| **Row-level granularity** | Can filter, inspect, or reject individual changes |

### Disadvantages

| Drawback | Description |
|----------|-------------|
| **Requires integration** | Must attach session to connection in application |
| **Thread-local state** | Sessions bound to creating thread |
| **Some operations missed** | PRAGMA, ATTACH, VACUUM not captured |
| **Memory overhead** | Tracks pending changes until changeset generated |
| **Lifecycle management** | Must handle session enable/disable around transactions |

## Why SyndDB Uses Changesets

SyndDB's architecture requires validators to verify operations, not just reconstruct state. Key requirements:

### 1. SQL Operations as Audit Trail

From the [SPEC](../SPEC.md):

> "SQL operations themselves become the verifiable audit trail"

Validators need to see logical operations to verify business rules (e.g., "withdrawals don't exceed balance"). WAL pages are opaque—you'd need to parse SQLite's internal B-tree format to extract logical changes.

### 2. Cross-Architecture Determinism

Validators may run on different hardware than the application. Changesets are architecture-independent (logical values), while WAL pages may have different layouts due to:

- Endianness differences
- Alignment/padding variations
- Page size configurations

### 3. Compact Wire Format

SyndDB's CBOR wire format achieves ~40% size reduction. Changesets contain only changed values, while WAL frames contain full 4KB pages even for single-column updates.

### 4. Schema Change Detection

SyndDB triggers immediate snapshots on schema changes to ensure validators can reconstruct the database. The Session Extension detects DDL operations directly; WAL replication would require parsing page content to detect schema changes.

### 5. Sequencing and Signing

Every message (changeset or snapshot) gets:

- A monotonic sequence number from the sequencer
- A COSE_Sign1 signature
- Optional TEE attestation

WAL frames don't have this metadata. Adapting Litestream would require significant additions to provide sequencing and signing.

## Extracting Logical Changes from WAL

Could you parse WAL to get changeset-like output? Yes, but it's complex:

```
WAL Frame → Page Content → B-tree Parsing → Row Extraction → Logical Diff
```

This requires understanding:

1. **WAL frame headers** - Documented, manageable
2. **SQLite page format** - B-tree interior/leaf pages, overflow pages
3. **B-tree structure** - Cell format, pointer maps, free lists
4. **Schema mapping** - Column types from sqlite_schema
5. **Change detection** - Diff before/after page content

The Session Extension already does this correctly by hooking into SQLite's internal change tracking. Reimplementing it from the physical layer is substantial work with potential for subtle bugs.

## When to Use Each Approach

### Use WAL-Based (Litestream) When:

- Disaster recovery / backup is the primary goal
- You need zero application code changes
- Validators just reconstruct state (don't verify operations)
- Point-in-time recovery to arbitrary moments is needed
- You're replicating to read-only replicas

### Use Changeset-Based (Session Extension) When:

- Validators need to audit logical operations
- Cross-architecture determinism is required
- Compact payloads matter
- You need schema change awareness
- Operations need sequencing and signing

## Hybrid Approach

These approaches can coexist for different purposes:

```
Application (TEE)
├── SQLite Database
├── SyndDB Client (changesets) → Sequencer → DA Layer → Validators
└── Litestream (WAL) → S3 → Disaster Recovery (not for validators)
```

Use changesets for the validator verification path and WAL for application-level backup. However, this adds operational complexity.

## Conclusion

SyndDB's requirements—auditable operations, deterministic replay, compact payloads, sequencing, and signing—align better with changeset-based replication. WAL-based tools like Litestream are excellent for backup and disaster recovery but don't provide the logical visibility that validators need.

The Session Extension's complexity (thread-local state, lifecycle management) is the cost of getting logical change capture. This complexity is bounded and well-understood, whereas parsing WAL to extract logical changes would be reimplementing Session Extension functionality from scratch.

## References

- [SQLite Session Extension](https://www.sqlite.org/sessionintro.html)
- [SQLite WAL Mode](https://www.sqlite.org/wal.html)
- [Litestream Documentation](https://litestream.io/how-it-works/)
- [SyndDB SPEC](../SPEC.md)
