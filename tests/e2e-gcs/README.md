# GCS E2E Tests

End-to-end tests for the SyndDB GCS storage layer integration.

## Overview

These tests validate the full flow:
1. Customer app sends changesets to sequencer
2. Sequencer publishes batches to GCS
3. Validators fetch and sync from GCS
4. Multi-validator consistency is verified

## Running Tests

### Emulator Mode (CI/Local)

Uses `fake-gcs-server` as a local GCS emulator. No GCP credentials required.

```bash
cargo test -p synddb-e2e-gcs
```

### Real GCS Mode (Local Only)

Tests against an actual GCS bucket. Useful for validating real GCS behavior before deploying to Confidential Space.

> **Note:** This mode uses service account keys which are long-lived credentials intended for local testing only. In production, use Workload Identity instead. Keys should be rotated regularly and never committed to version control.

#### Setup

Run the setup script to create the bucket, service account, and credentials:

```bash
# Uses synd-db-testing project by default
./tests/e2e-gcs/setup-real-gcs.sh

# Or specify a different project
./tests/e2e-gcs/setup-real-gcs.sh your-project-id
```

The script is idempotent - safe to run multiple times. It will:
- Create bucket `synddb-e2e-test` with 7-day lifecycle policy
- Create service account `synddb-e2e-test`
- Grant `roles/storage.objectAdmin` on the bucket
- Download credentials to `tests/e2e-gcs/.credentials/service-account.json`

#### Running Tests

```bash
GCS_BUCKET=synddb-e2e-test \
GOOGLE_APPLICATION_CREDENTIALS=tests/e2e-gcs/.credentials/service-account.json \
  cargo test -p synddb-e2e-gcs test_gcs_real -- --ignored
```

Each test run creates data under a unique prefix (`sequencer-test-<timestamp>`). Old test data is automatically deleted after 7 days by the bucket lifecycle policy.

#### Manual Cleanup

To delete test data immediately:

```bash
gsutil -m rm -r gs://synddb-e2e-test/sequencer-test-<timestamp>/
```

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                     Docker Compose Network                       │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌───────────┐   ┌─────────────┐   ┌───────────────────────┐     │
│  │ fake-gcs  │◄──│ init-bucket │   │                       │     │
│  │ (emulator)│   │ (creates    │   │    e2e_assertions     │     │
│  │           │   │  bucket)    │   │    (test runner)      │     │
│  └─────▲─────┘   └─────────────┘   └───────────┬───────────┘     │
│        │                                       │                 │
│        │ GCS API                               │ HTTP            │
│        │                                       ▼                 │
│  ┌─────┴───────────────────────────────────────────────────┐     │
│  │                                                         │     │
│  │               ┌─────────────┐                           │     │
│  │               │  sequencer  │◄──── customer_app         │     │
│  │               │  (GCS pub)  │      (changesets)         │     │
│  │               └──────┬──────┘                           │     │
│  │                      │                                  │     │
│  │                      │ publishes batches                │     │
│  │                      ▼                                  │     │
│  │               GCS Bucket                                │     │
│  │              (synddb-test)                              │     │
│  │                      │                                  │     │
│  │                      │ fetches batches                  │     │
│  │                      ▼                                  │     │
│  │          ┌───────────┴───────────┐                      │     │
│  │          │                       │                      │     │
│  │     ┌────▼────┐           ┌──────▼─────┐                │     │
│  │     │validator│           │ validator2 │                │     │
│  │     │(GCS fet)│           │ (GCS fet)  │                │     │
│  │     └─────────┘           └────────────┘                │     │
│  │                                                         │     │
│  └─────────────────────────────────────────────────────────┘     │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

## Files

- `docker-compose.yml` - Main compose file (emulator mode)
- `docker-compose.real-gcs.yml` - Override for real GCS mode
- `runner/` - Rust test runner crate
- `setup-real-gcs.sh` - Idempotent setup script for real GCS testing
- `README.md` - This file

## Environment Variables

### Emulator Mode
All configuration is embedded in `docker-compose.yml`. No environment variables needed.

### Real GCS Mode
| Variable | Required | Description |
|----------|----------|-------------|
| `GCS_BUCKET` | Yes | GCS bucket name |
| `GOOGLE_APPLICATION_CREDENTIALS` | Yes | Path to service account JSON |
| `TEST_RUN_ID` | No | Unique prefix for test data (default: timestamp) |
