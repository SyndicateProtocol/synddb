# Message Passing Bridge

A system for validating and executing cross-chain messages with configurable trust models.

## Overview

The Message Passing Bridge enables applications to send typed messages to validators, who validate them against Bridge-defined rules before signing and submitting for on-chain execution.

```
Application
    |
    | HTTP POST /messages
    v
Validator(s)
    |
    | 1. Validate message type & schema
    | 2. Apply custom validation rules
    | 3. Sign with TEE-protected key
    | 4. Publish to storage layer
    | 5. Submit signature to Bridge
    v
Bridge Contract
    |
    | Aggregate signatures, enforce threshold, execute
    v
Target Contract
```

## Documentation

- **[SPEC.md](SPEC.md)** - Complete technical specification
- **[PLAN_CONTRACTS.md](PLAN_CONTRACTS.md)** - Bridge contract implementation plan
- **[PLAN_VALIDATORS.md](PLAN_VALIDATORS.md)** - Validator implementation plan

## Repository Structure

```
synd-bridge/
├── contracts/              # Solidity smart contracts
├── crates/
│   ├── synddb-validator/   # Primary/Witness validator service
│   ├── synddb-chain-monitor/ # Blockchain event monitoring
│   └── synddb-shared/      # Shared types and utilities
├── tests/
│   └── confidential-space/ # TEE attestation testing
├── SPEC.md                 # Technical specification
├── PLAN_CONTRACTS.md       # Contract implementation plan
├── PLAN_VALIDATORS.md      # Validator implementation plan
└── README.md               # This file
```

## Key Concepts

**Validators** operate in one of two modes:
- **Primary Validator**: Receives messages directly from applications via HTTP
- **Witness Validator**: Reads messages from storage layer for independent verification

**Bridge Contract** is the trust anchor that:
- Maintains registry of allowed message types
- Aggregates validator signatures with threshold enforcement
- Executes validated messages via modular pre/post hooks

**Storage Layer**: Validators publish messages to Arweave, IPFS, or GCS for audit trails and Witness discovery.

## Development

```bash
# Build all crates
cargo build --workspace

# Run tests
cargo test --workspace

# Run CI checks
cargo +nightly fmt --all --check && \
cargo clippy --workspace --all-targets --all-features && \
cargo machete
```

## Requirements

- Rust 1.90.0 or later
- Foundry (for contract development)

## License

MIT License - see LICENSE file for details
