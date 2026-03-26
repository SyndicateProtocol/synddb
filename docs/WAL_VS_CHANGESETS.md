# Design Doc: WAL vs Changeset Replication

**Status:** Draft - Open for Discussion
**Author:** (auto-generated)
**Last Updated:** 2024-12

## Context

SyndDB currently uses changeset-based replication via the SQLite Session Extension. This document evaluates whether WAL-based replication (e.g., Litestream) would be a better approach, and explores hybrid options.

### What Prompted This Discussion

- Litestream is a mature, well-maintained tool for SQLite replication
- WAL-based replication requires zero application integration
- Session Extension has lifecycle complexity (thread-local state, enable/disable timing)
- Question: Are we overcomplicating capture when simpler tools exist?

## Current Architecture

```
Application (TEE)
└── SQLite + Session Extension
    └── Changesets → HTTP → Sequencer → COSE Sign → DA Layer → Validators
```

**Key characteristics:**
- Logical changes (INSERT/UPDATE/DELETE with values)
- In-process, same TEE as application
- Thread-local session state
- Automatic schema change detection triggers snapshots
- ~250 lines in `snapshot_sender.rs` + session integration

## Options Under Consideration

### Option A: Keep Current (Changeset-Based)

Continue using SQLite Session Extension for change capture.

### Option B: Replace with Litestream (WAL-Based)

Use Litestream or similar WAL replication, adding sequencing/signing layer.

### Option C: Hybrid Approach

Use changesets for validator path, add Litestream for disaster recovery.

### Option D: Custom WAL Parser

Build our own WAL-to-logical-changes converter for more control.

---

## Technical Comparison

| Aspect | WAL-Based | Changeset-Based |
|--------|-----------|-----------------|
| **Capture level** | Physical (4KB pages) | Logical (row changes) |
| **Integration** | External daemon | In-process library |
| **What's captured** | Everything (data, pragmas, vacuum) | INSERT/UPDATE/DELETE only |
| **Auditability** | Opaque bytes | Inspectable operations |
| **Payload size** | Larger (full pages) | Smaller (changed values) |
| **Determinism** | Architecture-dependent | Architecture-independent |
| **Complexity location** | Checkpoint coordination | Session lifecycle |
| **Maturity** | Litestream is battle-tested | Session Extension is SQLite core |
| **Inversion/undo** | Not possible (forward-only) | Native support via `sqlite3changeset_invert()` |

### WAL-Based: Detailed Analysis

**How it works:**
1. SQLite writes changes to `-wal` file
2. External daemon monitors WAL for new frames
3. Copies frames to replica storage before checkpoint
4. Recovery = snapshot + WAL replay

**What Litestream provides:**
- S3, GCS, Azure, SFTP backend support
- Generation-based WAL sequencing
- Point-in-time recovery
- Automatic snapshot scheduling

**What Litestream lacks:**
- Global monotonic sequence numbers
- Cryptographic signing (COSE_Sign1)
- TEE attestation integration
- Logical change visibility

**Effort to adapt Litestream:**
- Fork and modify, or wrap with signing layer
- Add sequence number assignment (per-frame? per-commit?)
- Integrate TEE attestation
- Either: validators understand WAL, or parse WAL → logical changes

### Changeset-Based: Detailed Analysis

**How it works:**
1. Attach session to SQLite connection
2. Session hooks into SQLite's change tracking
3. Generate changeset blob containing logical changes
4. Changeset is deterministic and architecture-independent

**Current pain points:**
- Thread-local state requires careful lifecycle management
- Must enable/disable session around certain operations
- Some operations not captured (PRAGMA, ATTACH, VACUUM)
- Memory overhead from tracking pending changes

**What works well:**
- Validators see exactly what changed
- Compact payloads (only changed columns)
- Schema change detection is automatic
- Deterministic replay across architectures

---

## Key Decision Factors

### 1. Validator Verification Model

**Question:** Do validators need to see logical operations, or just reconstruct state?

| If validators need to... | Then use... |
|--------------------------|-------------|
| Verify "balance >= withdrawal" | Changesets (logical) |
| Check "no self-trading" | Changesets (logical) |
| Just replay to same state | Either works |
| Audit specific row changes | Changesets (logical) |

**Current SPEC says:** "SQL operations themselves become the verifiable audit trail"

This implies validators inspect operations, not just replay them. **Does this requirement still hold?**

### 2. Cross-Architecture Determinism

WAL pages may differ across:
- Endianness (big vs little endian)
- Alignment/padding
- Page size configuration
- SQLite compile options

Changesets are architecture-independent by design.

**Question:** Will validators always run on identical architecture to the application?

### 3. Payload Size

Rough comparison for a single-column UPDATE:
- WAL: 4KB page (minimum)
- Changeset: ~50-200 bytes (column value + metadata)

**Question:** Is bandwidth/storage cost a significant concern?

### 4. Operational Complexity

| Approach | Application complexity | Infrastructure complexity |
|----------|------------------------|---------------------------|
| Changesets | Session lifecycle | None (in-process) |
| Litestream | None | Sidecar daemon, checkpoint coordination |
| Hybrid | Session lifecycle | Sidecar daemon |

**Question:** Where do we prefer complexity to live?

### 5. What Operations Need Capturing?

| Operation | Changeset captures | WAL captures |
|-----------|-------------------|--------------|
| INSERT/UPDATE/DELETE | Yes | Yes |
| Schema changes (DDL) | Yes (triggers snapshot) | Yes (in pages) |
| PRAGMA changes | No | Yes |
| VACUUM | No | Yes |
| ATTACH/DETACH | No | Yes |

**Question:** Do we need to capture PRAGMAs or VACUUM?

### 6. Changeset Inversion

**This is a capability unique to changesets that WAL cannot provide.**

The Session Extension provides `sqlite3changeset_invert()` to reverse any changeset:

```c
int sqlite3changeset_invert(
  int nIn, const void *pIn,      // Input changeset
  int *pnOut, void **ppOut       // OUT: Inverse of input
);
```

**How inversion works:**
- **INSERT** becomes **DELETE** (removes the inserted row)
- **DELETE** becomes **INSERT** (re-inserts the deleted row with original values)
- **UPDATE** swaps old/new values (reverts to previous column values)

If changeset `C+` is the inverse of `C`, then applying `C` followed by `C+` leaves the database unchanged.

**Why WAL cannot support inversion:**

WAL is a forward-only, append-only logging mechanism:

1. **No logical operations**: WAL contains raw page images, not row-level changes. There's no concept of "the row that was inserted" - just binary page data.

2. **Checkpointing destroys history**: When WAL frames are checkpointed back to the main database, the WAL is truncated or overwritten. Previous states are not preserved for reversal.

3. **Undo requires full restore**: To "undo" with WAL, you must restore from a previous snapshot. There's no incremental inverse operation.

**Benefits of inversion for SyndDB:**

| Use Case | How Inversion Helps |
|----------|---------------------|
| **Validator rollback** | If invalid state detected at seq N, apply inverse of changeset N to revert |
| **Dispute resolution** | Surgically revert specific transactions without full restore |
| **Optimistic execution** | Apply tentatively, roll back if sequencer rejects/reorders |
| **Point-in-time recovery** | Store changesets + inverses for bidirectional replay |
| **Testing** | Apply changes, verify, then revert - no database reset needed |

**Example: Validator rollback scenario**
```
Sequence 100: Changeset C (valid)
Sequence 101: Changeset D (later found to violate constraint)
Sequence 102: Changeset D_inverse (surgical rollback)
Sequence 103: Changeset E (corrected operation)
```

With WAL, the validator would need to restore a full snapshot from before sequence 101.

**Is inversion worth the Session Extension complexity?**

Strong yes when:
- Validators may need to propose rollbacks for invalid transitions
- System requires point-in-time recovery without full snapshots
- Optimistic execution patterns are used (apply then verify)

Less critical when:
- Forward-only replication is sufficient
- Full checkpoint restore is acceptable for all rollback scenarios

**Question:** Do we anticipate validators needing fine-grained rollback, or is snapshot restore acceptable?

---

## Hybrid Architecture (Option C)

```
Application (TEE)
├── SQLite Database
│
├── SyndDB Client (changesets)
│   └── Sequencer → DA Layer → Validators
│   (validator verification path)
│
└── Litestream (WAL)
    └── S3/GCS → Disaster Recovery
    (application backup, not for validators)
```

**Use cases for WAL backup:**
- Application crashes before sending changesets
- Need to recover local state quickly
- Debug/forensics on raw database state
- Belt-and-suspenders redundancy

**Downsides:**
- Two replication systems to operate
- WAL backup not useful for validator bootstrap
- Additional infrastructure (Litestream sidecar)

---

## Effort Estimates

| Option | Estimated Effort | Risk Level |
|--------|------------------|------------|
| A: Keep current | None | Low |
| B: Replace with Litestream | 2-4 weeks + ongoing maintenance | High |
| C: Add Litestream for DR | 1 week | Low |
| D: Custom WAL parser | 4-8 weeks | Very High |

**Option B breakdown:**
- Fork Litestream or build wrapper: 1 week
- Add sequencing/signing: 1 week
- Modify validators to handle WAL or parse to logical: 1-2 weeks
- Testing and edge cases: 1 week

**Option D risks:**
- SQLite page format is internal/undocumented
- B-tree parsing is complex (overflow pages, pointer maps)
- Essentially reimplementing Session Extension from scratch

---

## Open Questions for Discussion

### Architecture Questions

1. **Do validators need logical auditability?**
   If validators only replay state (not inspect operations), WAL becomes viable.

2. **Is cross-architecture determinism required?**
   If app and validators always share architecture, WAL determinism concerns go away.

3. **Should snapshots be WAL-based or remain as full DB copies?**
   Current: full SQLite file. Could be: WAL generation + frames.

### Operational Questions

4. **What are the actual pain points with Session Extension today?**
   Thread-local state? Memory overhead? Something else?

5. **Is application-level disaster recovery (Option C) valuable?**
   Do we have a recovery gap if app crashes before changeset send?

6. **Where should complexity live - application or infrastructure?**
   Session Extension = app complexity. Litestream = infra complexity.

### Performance Questions

7. **Is changeset generation a performance bottleneck?**
   Have we measured Session Extension overhead?

8. **Is payload size a concern for DA layer costs?**
   WAL would increase payload size significantly.

### Future Questions

9. **Do we anticipate needing PRAGMA/VACUUM capture?**
   Currently not captured by changesets.

10. **Would WAL simplify the FFI story for other languages?**
    External daemon vs in-process library integration.

---

## Experiments We Could Run

### Experiment 1: Measure Session Extension Overhead
- Benchmark with/without session attached
- Measure memory usage during large transactions
- Identify actual (not theoretical) pain points

### Experiment 2: Prototype Litestream Integration
- Run Litestream alongside existing system
- Measure WAL payload sizes vs changeset sizes
- Test checkpoint coordination in TEE environment

### Experiment 3: WAL-to-Changeset Feasibility
- Prototype parsing WAL frames
- Assess complexity of extracting logical changes
- Determine if this is 2 weeks or 2 months of work

---

## Recommendation

**Tentative recommendation: Option A (keep current) with possible Option C (add Litestream for DR)**

Rationale:
- Validator auditability requirement favors changesets
- Changeset inversion enables surgical rollback that WAL cannot provide
- Session Extension complexity is bounded and understood
- WAL adaptation would be significant effort for unclear benefit
- Litestream for DR is low-effort and provides safety net

**However, this should be validated against the open questions above.**

---

## References

- [SQLite Session Extension](https://www.sqlite.org/sessionintro.html)
- [SQLite WAL Mode](https://www.sqlite.org/wal.html)
- [Litestream How It Works](https://litestream.io/how-it-works/)
- [sqlite3changeset_invert() API](https://sqlite.org/session/sqlite3changeset_invert.html)
- [SyndDB SPEC](../SPEC.md)
- Current implementation: `crates/synddb-client/src/session.rs`, `snapshot_sender.rs`
