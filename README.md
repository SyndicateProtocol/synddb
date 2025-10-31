# SyndDB
Infinitely scaling Syndicate Network via the magic of SQLite

## Documentation Guide

- **[SPEC.md](SPEC.md)** - Complete technical specification covering architecture, terminology, and design decisions
- **[PLAN_CORE.md](PLAN_CORE.md)** - Implementation plan for SyndDB Core (sequencer, read replicas, state management)
- **[PLAN_EXTENSIONS.md](PLAN_EXTENSIONS.md)** - Implementation plan for the extension framework infrastructure
- **[crates/synddb-core/README.md](crates/synddb-core/README.md)** - Getting started with the SyndDB Core implementation

For a detailed understanding of the overall architecture, node types, and state management, see [SPEC.md](SPEC.md).

**Note:** Developer guides for building custom extensions will be added in future releases. PLAN_EXTENSIONS.md focuses on implementing the extension framework itself, not on how to use it.

## Quick Start

### Prerequisites

- Rust 1.70+ (install from [rustup.rs](https://rustup.rs))
- SQLite 3.35+ (bundled with rusqlite, no separate install needed)

### Installation

```bash
# Clone the repository
git clone https://github.com/SyndicateProtocol/SyndDB.git
cd SyndDB

# Build all workspace crates
cargo build --release

# Run tests to verify installation
cargo test
```

### Running the Example

```bash
# Run the basic usage example
cargo run --package synddb-core --example basic_usage

# Query the example database
sqlite3 example.db "SELECT name, email, balance FROM users ORDER BY balance DESC;"
```

### Development Setup

1. **Install git hooks (required for contributors):**
   ```bash
   # Install pre-commit hooks for automatic formatting
   ./.githooks/install.sh
   ```

   The hooks will automatically format Rust and Solidity code before each commit. See [.githooks/README.md](.githooks/README.md) for details.

2. **Configure your environment (recommended):**
   ```bash
   # Create .env file for local development (optional, has defaults)
   cp .env.example .env
   # Edit .env with your local settings
   ```

   Note: The `.env` file is the recommended way to configure SyndDB for local development. It's automatically loaded and never committed to git. For production deployments, use environment variables directly.

3. **Build and test:**
   ```bash
   # Build specific crate
   cargo build --package synddb-core

   # Run tests with output
   cargo test --package synddb-core -- --nocapture

   # Format code (done automatically by git hooks)
   cargo fmt --all

   # Run linter
   cargo clippy --package synddb-core -- -D warnings
   ```

4. **View documentation:**
   ```bash
   cargo doc --package synddb-core --open
   ```

## CI/CD

### Automated Testing

Currently the project uses local testing workflows. Run these commands before submitting PRs:

```bash
# Run full test suite
cargo test

# Check formatting
cargo fmt --all -- --check

# Run clippy lints
cargo clippy --all-targets -- -D warnings

# Build all packages
cargo build --release
```

### Planned CI Integration

GitHub Actions workflows will be added for:
- Automated testing on push/PR
- Code formatting checks
- Clippy linting
- Build verification across platforms
- Performance benchmarking
- Documentation generation

## Project Structure

```
synddb/
├── crates/
│   ├── synddb-core/        # Core database engine and extensions
│   ├── synddb-sequencer/   # Sequencer node implementation
│   ├── synddb-replica/     # Read replica node implementation
│   ├── synddb-contracts/   # Smart contracts for state publication
│   ├── synddb-storage/     # IPFS/Arweave storage providers
│   └── synddb-utils/       # Shared utilities
├── SPEC.md                 # Technical specification
├── PLAN_CORE.md           # Core implementation plan
└── PLAN_EXTENSIONS.md     # Extension development guide
```
