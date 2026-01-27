# E2E Smoke Tests

End-to-end integration tests that validate the full SyndDB pipeline.

## Architecture

```
┌──────────────┐     changesets     ┌───────────┐
│ customer_app │ ─────────────────► │ sequencer │
│  (benchmark) │                    │           │
└──────────────┘                    └─────┬─────┘
                                          │ storage messages
                                          ▼
┌──────────────┐     fetch/sync     ┌───────────────┐
│  validator   │ ◄────────────────  │ local storage │
└──────────────┘                    └───────────────┘
        │
        │ HTTP API
        ▼
┌──────────────┐
│e2e_assertions│ ──► exit 0 (pass) / exit 1 (fail)
└──────────────┘
```

## Components

| Service | Description |
|---------|-------------|
| `sequencer` | Signs changesets and publishes to local storage |
| `validator` | Syncs messages from sequencer via HTTP, validates signatures |
| `validator2` | Second validator for multi-validator sync testing |
| `customer_app` | Orderbook benchmark generating ~1000 changesets over 20s |
| `e2e_assertions` | Rust test runner (`tests/e2e/runner`) that validates the pipeline |

## Tests

- **sequencer_received** - Sequencer received and processed messages
- **validator_synced** - Validator synced messages from storage
- **sync_consistency** - Validator caught up with sequencer (within tolerance)
- **storage_fetch** - Storage message fetch API returns valid signed messages
- **multi_validator** - Multiple validators sync to the same state
- **snapshot** - Snapshot functionality works correctly

## Running Locally

```bash
docker compose -f tests/e2e/docker-compose.yml up --build --abort-on-container-exit --exit-code-from e2e_assertions
```

To rebuild without cache after code changes:

```bash
docker compose -f tests/e2e/docker-compose.yml build --no-cache && \
docker compose -f tests/e2e/docker-compose.yml up --abort-on-container-exit --exit-code-from e2e_assertions
```

To clean up after running:

```bash
docker compose -f tests/e2e/docker-compose.yml down --volumes --remove-orphans
```

## Build Optimization

All services use a single multi-stage Dockerfile (`docker/local.Dockerfile`) with different build targets.
Docker BuildKit builds the shared `builder` stage once and reuses it for all service targets,
reducing build time compared to separate Dockerfiles.

Available targets:
- `sequencer` - SyndDB sequencer service
- `validator` - SyndDB validator service
- `customer-app` - Orderbook benchmark
- `e2e` - E2E test runner
- `stress-test` - Session stress test

## CI

Runs on every PR and push to `main` via `.github/workflows/e2e.yml`. The workflow:

1. Builds all containers (shared builder stage compiled once)
2. Runs the full pipeline
3. Exits with the `e2e_assertions` container's exit code
4. On failure, dumps all service logs for debugging

## Files

- `docker-compose.yml` - Orchestrates all services
- `runner/` - Rust test runner source code
- `../../docker/local.Dockerfile` - Multi-stage Dockerfile for all services
