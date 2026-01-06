# CLAUDE.md

This is the guide for Claude Code when working with the SyndDB monorepo.

## Project Overview

SyndDB is a SQLite replication system for blockchain-based applications. It captures SQLite changesets, sequences them through a trusted sequencer, and publishes to data availability layers for deterministic state reconstruction.

**Architecture**: Two-VM security model where the application (VM1) captures changes and the sequencer (VM2) signs them with isolated keys.

**Status**: Greenfield codebase not yet in production. No backwards compatibility requirements - feel free to make breaking changes to APIs, data formats, or interfaces as needed.

## Crate Structure

```
crates/
├── synddb-client/        # Client library for changeset capture (Rust + FFI)
├── synddb-sequencer/     # Message ordering and signing service
├── synddb-validator/     # State validation and replica reconstruction
├── synddb-chain-monitor/ # Blockchain event monitoring (WebSocket/RPC)
├── synddb-shared/        # Shared types and utilities across crates
└── synddb-benchmark/     # Orderbook simulator for performance testing
```

## Development Patterns

### Configuration
All configs derive `Serialize`, `Deserialize`, and clap `Parser`. Support both CLI args and env vars:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Parser)]
pub struct Config {
    #[arg(long, env = "VAR_NAME", default_value = "value")]
    pub field: Type,

    #[arg(long, env = "DURATION", value_parser = parse_duration)]
    #[serde(with = "humantime_serde")]
    pub timeout: Duration,
}
```

### Error Handling
- `anyhow::Result<T>` for fallible operations
- `thiserror::Error` for custom error types
- Add `.context()` at system boundaries

### Async Patterns
- Tokio runtime with `current_thread` for background tasks
- `crossbeam-channel` for sync-async boundaries
- `async_trait` for async trait methods

### Testing
- Inline `#[cfg(test)] mod tests` in source files
- Builder methods for test configs: `Config::with_signing_key()`
- In-memory SQLite: `Connection::open_in_memory()`
- Don't import libraries for specific tests, import them at the top level instead

### Test Modification Policy
Treat existing tests as immutable. If an existing test fails or seems to need modification, ask for confirmation before changing it—the test may be catching a regression. This does not apply to new tests written as part of the current work, meaning the new content introduced in the current branch or PR.

### Project structure
- Do not re-export dependencies from `crates/` in mod.rs or lib.rs files. Refer to the full path instead.

### Incremental Commits
When working on large features or refactors, commit incrementally as you complete each todo item. Do not wait until the entire feature is complete. This:
- Preserves working states you can revert to
- Makes code review easier
- Reduces risk of losing work
- Creates a clear history of changes

### Git Staging
Avoid `git add -A`, `git add .`, or `git add --all`. These commands stage all modified files, which can accidentally include unrelated changes. Instead, stage only the specific files you modified for the current task:
```bash
git add path/to/file1.rs path/to/file2.rs
```

Similarly, avoid `git restore .` or `git checkout .` to discard changes. Other files in the working directory may contain work from parallel sessions that should be preserved. Only restore specific files you intend to discard.

### Force Pushing
Never force push (`git push --force` or `git push -f`) if there is any alternative. Force pushing rewrites history and can cause problems for collaborators who have already pulled the branch.

If you need to fix a prior commit (typo, missing file, etc.), do not use `git commit --amend` followed by force push. Instead, make the correction and create a new commit. This preserves history and avoids the need for force pushing.

### Hard Reset
Never run `git reset --hard` without asking for explicit confirmation first. Hard resets discard all uncommitted changes in the working directory, which can permanently delete work from parallel sessions. Always ask the user before running any variant of `git reset --hard`.

### Documentation (SPEC and PLAN files)
The `SPEC.md` and `PLAN_*.md` files document the specifications and implementation plans for each component. These files must be kept up to date with the current implementation:
- After making major implementation changes, update the corresponding SPEC and PLAN files to reflect the new state
- When adding new features or modifying existing behavior, ensure the documentation matches the actual code
- Review these files before starting work on a component to understand the intended design

## Key Dependencies

| Category | Crates |
|----------|--------|
| Async | `tokio`, `async-trait` |
| Web | `axum`, `reqwest`, `tower` |
| Serialization | `serde`, `serde_json`, `humantime-serde` |
| Blockchain | `alloy` (provider-ws, rpc-types, sol-types, signers) |
| Database | `rusqlite` (bundled, session, hooks) |
| CLI | `clap` (derive) |
| Errors | `anyhow`, `thiserror` |
| Logging | `tracing`, `tracing-subscriber` |

## Feature Flags

- `synddb-client`: `chain-monitor`, `ffi`
- `synddb-sequencer`: `gcs`, `tee`, `celestia`, `eigenda`, `ipfs`, `arweave`

## CI Validation

This project relies heavily on GitHub Actions for CI. After pushing to a branch, check workflow status with the `gh` CLI:

```bash
# List recent workflow runs
gh run list --limit 5

# Check runs for a specific workflow
gh run list --workflow=rust-ci.yml

# View details of a specific run
gh run view <run-id>

# Watch a run in progress
gh run watch <run-id>

# View logs for a failed run
gh run view <run-id> --log-failed

# Manually trigger a workflow
gh workflow run <workflow-name> --ref <branch>

# Check all CI status checks for current PR
gh pr checks

# View PR details
gh pr view <pr-number>

# Check your current API rate limit
gh api rate_limit
```

**Rate Limits:** The `gh` CLI uses GitHub's REST and GraphQL APIs, which are subject to rate limits (5,000 requests/hour for authenticated users). Be judicious when checking workflow status:
- Avoid repeatedly running `gh run list` in a loop - each call consumes API quota
- Prefer `gh run watch <run-id>` to monitor a single run (it polls efficiently)
- For long-running workflows, check status infrequently rather than polling continuously
- If you hit rate limits, wait before retrying - the limit resets hourly

**Note:** When squash-merging PRs, ensure the commit message body does not contain `[skip ci]` - GitHub scans the entire message and will skip all workflows if found.

### Local CI Validation

Before committing, run the full CI validation locally:

```bash
# Full CI check (format, lint, unused deps, TOML format)
cargo +nightly fmt --all -- --unstable-features && \
cargo clippy --workspace --all-targets --all-features && \
cargo machete && \
taplo fmt "**/Cargo.toml"

# Run tests
cargo test --workspace

# Build all features
cargo build --workspace --all-features

# Check specific crate
cargo clippy -p synddb-sequencer --all-features
cargo test -p synddb-sequencer
```

**Required tools:**
- `rustup install nightly` (for fmt)
- `cargo install taplo-cli` (TOML formatting)
- `cargo install cargo-machete` (unused dependency detection)

## Justfile

The project uses [just](https://github.com/casey/just) as a command runner for local development. The justfile is the single source of truth for all development defaults (Anvil keys, contract addresses, ports).

**When to use:** Both local development and CI. The justfile is the single source of truth - CI jobs call the same recipes you run locally.

**Quick start:**
```bash
just              # Show all available commands
just dev          # Start full local environment (Anvil + contracts + sequencer)
just check        # Run all CI checks locally
just test         # Run tests
```

**Modules:** Recipes are organized into modules for contracts and examples:
```bash
just contracts::build    # Build Solidity contracts
just contracts::test     # Run contract tests
just examples::price-oracle      # Run price oracle example
```

**CI recipes:** Integration tests that start/stop services:
```bash
just stress-test         # Run stress test with sequencer
just client-integration  # Run client integration tests
just fuzz-ci             # Run fuzzer with CI iterations
```

### Language Features Used

| Feature | Purpose | Example |
|---------|---------|---------|
| `set shell` | Bash strict mode for all recipes | `set shell := ["bash", "-euo", "pipefail", "-c"]` |
| `set dotenv-load` | Load `.env` for optional overrides | Users can create `.env` to customize defaults |
| `set export` | Export all variables as env vars | Variables available to all recipes |
| `mod` | Organize recipes into modules | `mod contracts 'contracts/mod.just'` |
| `[group()]` | Group recipes in `just --list` | `[group('dev')]`, `[group('test')]` |
| `[confirm()]` | Require confirmation for destructive ops | `[confirm('Remove all data?')]` |
| Variables | Single source of truth for config | `anvil_key_0 := "ac097..."` |

### Configuration

All local dev defaults are defined in the justfile. No `.env` file is needed for basic development. To override any value, create a `.env` file (gitignored):

```bash
# .env (optional - for local overrides only)
RUST_LOG=debug
sequencer_port=9000
```

**Reference:** [Just Manual](https://just.systems/man/en/)

## Code Style

- No emojis in code or comments unless explicitly requested
- Prefer editing existing files over creating new ones
- Keep solutions minimal - avoid over-engineering
- Use `tracing` macros (`info!`, `warn!`, `debug!`, `error!`) for logging
- Structured logging with fields: `info!(sequence = seq, "Message")`
- Ethereum address naming: EOA (wallet) addresses use `_ADDRESS` suffix, contract addresses use `_CONTRACT_ADDRESS` suffix
- Doc comments: Use backticks around code identifiers, function calls, and technical terms to avoid `clippy::doc_markdown` warnings. This includes function calls like `` `keccak256(digest)` `` and words with internal capitalization like `` `SQLite` `` or `` `SyndDB` ``
- Do not add self-explanatory comments that do not clarify code. Below is an example of useless comments:

```rust
// Create shutdown channel
let (shutdown_tx, shutdown_rx) = watch::channel(false);

// Create DA fetcher based on config
let fetcher = create_fetcher(&config).await?;

// Create validator
let mut validator = Validator::new(&config, fetcher.clone(), shutdown_rx.clone())?;
```

## Common Workflows

### Adding a new endpoint (synddb-sequencer)
1. Add request/response types in `http_api.rs`
2. Add handler function
3. Register route in `create_router()`
4. Add tests

### Adding a new publisher (synddb-sequencer)
1. Implement `DAPublisher` trait in `publish/`
2. Add feature flag to `Cargo.toml`
3. Wire up in `main.rs` with feature gate

### Adding chain monitor handler (synddb-client)
1. Implement `MessageHandler` trait
2. Register with `ChainMonitor::add_handler()`

## Security Model

### Signing Key Management
Both sequencers and validators generate ephemeral signing keys at startup. This is intentional for TEE isolation:

- **Do NOT** manage signing keys via GCP Secret Manager or environment variables
- **Do NOT** persist keys between service restarts
- Keys are generated fresh inside the TEE on each startup

When a service restarts with a new key, a new TEE attestation proof must be generated via the SP1 zkVM and submitted to the bridge contract. This registers the new public key on-chain, allowing the bridge to verify signatures from the new instance. This is the intended deployment model - keys are bound to TEE instances, not externally managed.

### TEE Boundary Considerations
The sequencer and validator run inside TEEs (Trusted Execution Environments). When suggesting refactors or architectural changes to these Rust components:

- **Do NOT** move security-critical logic out of sequencer/validator without explicit approval
- **Do NOT** extract signing, validation, or key management into separate services
- Any logic that handles cryptographic operations must remain inside the TEE boundary
- Be cautious when suggesting shared libraries or external dependencies that could execute outside the TEE

If a proposed change could affect the TEE security boundary, flag the implications and confirm before proceeding.

**Note:** Smart contracts (in `contracts/`) run on-chain, not inside TEEs. TEE boundary considerations do not apply to contract code.

### TEE Environment Variable Overrides
When adding or modifying environment variables for components that run in Confidential Space (sequencer, validator, price-oracle), you must update the `tee.launch_policy.allow_env_override` label in the corresponding Dockerfile. Confidential Space blocks all env vars by default—only those explicitly listed in this label can be set by the VM operator at runtime.

**Affected Dockerfiles:**
- `docker/reproducible/sequencer.Dockerfile:148` - Production sequencer
- `docker/reproducible/validator.Dockerfile:153` - Production validator
- `examples/price-oracle/Dockerfile:59` - Price oracle application
- `examples/price-oracle/validator.Dockerfile:46` - Price oracle validator
- `tests/confidential-space/Dockerfile:31` - Attestation sample

When adding a new env var:
1. Add the env var to the application code (config struct with `#[arg(env = "...")]`)
2. Update the `allow_env_override` label in the Dockerfile to include the new variable
3. Document the variable's purpose in a comment above the label (see `examples/price-oracle/Dockerfile:52-58` for the pattern)

## GCP Infrastructure

### Terraform Environments
Infrastructure is managed via Terraform in `deploy/terraform/environments/`:
- `staging/` - Test environment on Base Sepolia
- `prod/` - Production environment

### Updating Image Digests and Signatures
When updating Terraform configurations with new image digests and signatures, use the `get-image-info` script:

```bash
# Get digest and signature for an image tag
echo '{"image": "us-central1-docker.pkg.dev/synddb-infra/synddb/sequencer:latest"}' | \
  ./deploy/terraform/scripts/get-image-info.sh

# Get signature for a specific digest
echo '{"image": "us-central1-docker.pkg.dev/synddb-infra/synddb/sequencer@sha256:abc123..."}' | \
  ./deploy/terraform/scripts/get-image-info.sh
```

The script outputs JSON with `digest`, `signature`, and `found` fields. Use these values to update the corresponding Terraform variables.

**Requirements:** `oras` CLI must be installed.

### Deleting Infrastructure
Cloud Run v2 services have `deletion_protection = true` by default. To delete:

1. Set `deletion_protection = false` on the resource in Terraform
2. Run `terraform apply` to update the protection setting
3. Run `terraform destroy` to delete the infrastructure

Do NOT use `gcloud run services delete` as it causes Terraform state drift.
