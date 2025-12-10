# GCS Integration Implementation Plan

## Overview

Enable end-to-end testing of the GCS storage layer for the sequencer publish and validator read flow. This includes Docker-based local testing with `fake-gcs-server` and documentation for production GCP setup.

## Current State

- **Sequencer GCS Publisher** (`crates/synddb-sequencer/src/publish/gcs.rs`): Fully implemented, stores batches at `{prefix}/batches/{start:012}_{end:012}.json`
- **Validator GCS Fetcher** (`crates/synddb-validator/src/sync/providers/gcs.rs`): Updated to read batches (path aligned with sequencer)
- **Batch Support**: `DAFetcher` trait has batch methods, `BatchIndex` for efficient sync
- **E2E Tests**: Batch tests exist for HTTP fetcher, need GCS-specific tests

## Implementation Phases

### Phase 1: GCS E2E Docker Infrastructure

**Goal**: Create a docker-compose setup that tests the full GCS flow locally.

**Files to create**:
- `tests/e2e-gcs/docker-compose.yml`
- `tests/e2e-gcs/init-bucket.sh`

**Docker services**:
1. `fake-gcs` - GCS emulator (fsouza/fake-gcs-server)
2. `init-bucket` - Creates the test bucket before other services start
3. `sequencer` - With `PUBLISHER_TYPE=gcs`, `GCS_BUCKET=synddb-test`
4. `validator` - With `FETCHER_TYPE=gcs`, same bucket config
5. `validator2` - Second validator for multi-validator testing
6. `customer_app` - Generates changesets
7. `e2e_assertions` - Runs test assertions

**Key configuration**:
```yaml
# All GCS-using services need:
environment:
  STORAGE_EMULATOR_HOST: "http://fake-gcs:4443"
  GCS_BUCKET: "synddb-test"
  GCS_PREFIX: "sequencer"
```

**Bucket initialization**:
```bash
# init-bucket.sh
curl -X POST "http://fake-gcs:4443/storage/v1/b?project=test" \
  -H "Content-Type: application/json" \
  -d '{"name": "synddb-test"}'
```

### Phase 2: GCS E2E Test Assertions

**Goal**: Add GCS-specific test cases to verify the flow works correctly.

**Files to modify**:
- `tests/e2e/runner/src/tests/gcs.rs` (new)
- `tests/e2e/runner/src/tests/mod.rs`
- `tests/e2e/runner/src/runner.rs`

**Test cases**:
1. `test_gcs_sequencer_publishes` - Verify sequencer publishes to GCS
2. `test_gcs_validator_syncs` - Verify validator reads from GCS
3. `test_gcs_batch_structure` - Verify batch files have correct format
4. `test_gcs_multi_validator_consistency` - Both validators sync correctly

**Note**: Most existing batch tests should work since they test the DA layer abstraction. GCS-specific tests verify the actual GCS integration.

### Phase 3: GCS Setup Documentation

**Goal**: Document how to set up GCS for production deployment in GCP Confidential Space.

**Files to create**:
- `docs/gcs-setup.md`
- `scripts/gcs-setup.sh`

**Documentation contents**:
1. Prerequisites (GCP project, gcloud CLI)
2. Bucket creation with recommended settings
3. Service account creation with minimal permissions
4. Workload Identity binding for Confidential Space
5. Environment variable configuration
6. Verification steps

**Script functionality**:
```bash
./scripts/gcs-setup.sh <project-id> <bucket-name> <service-account>
```
- Creates GCS bucket with uniform bucket-level access
- Grants `roles/storage.objectAdmin` to service account
- Outputs required environment variables

### Phase 4: BatchConfig Implementation (Fast Follow)

**Goal**: Implement configurable batching between HTTP handlers and publishers.

**Files to create/modify**:
- `crates/synddb-sequencer/src/batcher.rs` (new)
- `crates/synddb-sequencer/src/config.rs`
- `crates/synddb-sequencer/src/main.rs`
- `crates/synddb-sequencer/src/lib.rs`

**BatchConfig struct**:
```rust
pub struct BatchConfig {
    /// Maximum messages per batch before flushing (default: 1, i.e., no batching)
    pub batch_size: usize,
    /// Maximum time to wait before flushing a partial batch (default: 1s)
    pub batch_interval: Duration,
    /// Enable zstd compression for batches (default: false)
    pub compress: bool,
}
```

**Batcher behavior**:
- Accumulates messages in memory
- Flushes when `batch_size` reached OR `batch_interval` elapsed
- Fire-and-forget mode: HTTP handler returns immediately, batch publishes async
- Thread-safe with `Arc<Mutex<>>` or channel-based design

**Config additions**:
```rust,no_run
#[arg(long, env = "BATCH_SIZE", default_value = "1")]
pub batch_size: usize,

#[arg(long, env = "BATCH_INTERVAL", default_value = "1s")]
pub batch_interval: Duration,

#[arg(long, env = "BATCH_COMPRESS", default_value = "false")]
pub batch_compress: bool,
```

## Dependency Graph

```
Phase 1 (Docker Infrastructure) ──┐
                                  ├──> Phase 2 (Test Assertions)
                                  │
Phase 3 (Documentation) ──────────┤
                                  │
                                  v
                          Phase 4 (BatchConfig)
```

Phases 1 and 3 can be done in parallel. Phase 2 depends on Phase 1. Phase 4 is a fast follow after GCS e2e is working.

## Testing Strategy

### Local Testing (Docker)
```bash
# Run GCS e2e tests
docker compose -f tests/e2e-gcs/docker-compose.yml up --build \
  --abort-on-container-exit --exit-code-from e2e_assertions
```

### CI Considerations
- GCS e2e tests should be `#[ignore]` by default or in a separate workflow
- They require Docker and take longer than unit tests
- Consider running on merge to main only, not on every PR

## Success Criteria

1. GCS e2e tests pass locally with fake-gcs-server
2. Sequencer publishes batches to GCS bucket
3. Validator reads and syncs from GCS bucket
4. Multi-validator consistency maintained
5. Documentation enables production GCS setup
6. BatchConfig allows configurable batching (fast follow)
