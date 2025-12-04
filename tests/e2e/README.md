# E2E Smoke Tests

End-to-end integration tests that validate the full SyndDB pipeline.

## Architecture

```
┌──────────────┐     changesets     ┌───────────┐
│ customer_app │ ─────────────────► │ sequencer │
│  (benchmark) │                    │           │
└──────────────┘                    └─────┬─────┘
                                          │ DA messages
                                          ▼
┌──────────────┐     fetch/sync     ┌───────────┐
│  validator   │ ◄────────────────  │ local DA  │
└──────────────┘                    └───────────┘
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
| `sequencer` | Signs changesets and publishes to local DA storage |
| `validator` | Syncs messages from sequencer via HTTP, validates signatures |
| `customer_app` | Orderbook benchmark generating ~1000 changesets over 20s |
| `e2e_assertions` | Rust test runner (`crates/synddb-e2e`) that validates the pipeline |

## Tests

- **sequencer_received** - Sequencer received and processed messages
- **validator_synced** - Validator synced messages from DA
- **sync_consistency** - Validator caught up with sequencer (within tolerance)
- **da_fetch** - DA message fetch API returns valid signed messages

## Running Locally

```bash
cd tests/e2e
docker compose up --build --abort-on-container-exit --exit-code-from e2e_assertions
```

To rebuild without cache after code changes:

```bash
docker compose build --no-cache && docker compose up --abort-on-container-exit --exit-code-from e2e_assertions
```

## CI

Runs on every PR and push to `main` via `.github/workflows/e2e.yml`. The workflow:

1. Builds all containers
2. Runs the full pipeline
3. Exits with the `e2e_assertions` container's exit code
4. On failure, dumps all service logs for debugging

## Files

- `docker-compose.yml` - Orchestrates all services
- `../../docker/e2e.Dockerfile` - Builds the test runner
- `../../crates/synddb-e2e/` - Test runner source code
