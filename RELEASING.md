# Release Process

This document describes the FFI library build and release process for SyndDB.

## Overview

SyndDB uses a single unified workflow (`build-ffi-libs.yml`) that handles three distinct scenarios:

### 1. Pull Requests (Dry Run)
- **Trigger**: PRs that touch client code
- **Platforms**: Linux x86_64, macOS ARM64
- **Builds**: Yes
- **Commits**: No
- **Artifacts**: Yes (7 days)
- **Purpose**: Verify FFI builds work before merge

### 2. Main Branch (Development)
- **Trigger**: Commits to `main` that touch client code
- **Platforms**: Linux x86_64, macOS ARM64
- **Builds**: Yes
- **Commits**: Yes (to `crates/synddb-client/libs/` with `[skip ci]`)
- **Artifacts**: Yes (7 days)
- **Purpose**: Keep libraries in sync for cross-language developers

### 3. Release Tags (Production)
- **Trigger**: Git tags matching `v*.*.*` (e.g., `v0.2.0`)
- **Platforms**: All 4 (Linux, macOS x64, macOS ARM64, Windows)
- **Builds**: Yes
- **Commits**: No
- **Artifacts**: Yes (90 days)
- **GitHub Release**: Yes (with checksums)
- **crates.io**: Yes (publishes package)
- **Purpose**: Production binaries for end users

**Key Design:** One workflow file, conditional logic based on `github.event_name` and `github.ref`

## Development Libraries

Libraries are automatically rebuilt and committed to the repository on every push to `main`:

**Location**: `crates/synddb-client/libs/`
```
libs/
├── linux-x64/
│   └── libsynddb_client.so
└── darwin-arm64/
    └── libsynddb_client.dylib
```

**Benefits:**
- Python/Node.js developers can use libraries without building Rust
- Libraries are always in sync with the code
- CI automatically updates them via `[skip ci]` commits

**How it works:**
1. Code pushed to `main`
2. GitHub Actions builds Linux + macOS ARM64 libs
3. Bot commits libraries back to repo with `[skip ci]`
4. Developers pull and use updated libraries

## Release Platforms

Release builds support all major platforms:

- **Linux x86_64**: `libsynddb_client.so`
- **macOS x86_64** (Intel): `libsynddb_client.dylib`
- **macOS ARM64** (M1/M2/M3): `libsynddb_client.dylib`
- **Windows x86_64**: `synddb_client.dll`

## Creating a Release

### Prerequisites

1. Ensure all tests pass on `main` branch
2. Update version numbers in relevant `Cargo.toml` files
3. Update `CHANGELOG.md` with release notes
4. Ensure `CARGO_REGISTRY_TOKEN` secret is configured in repository settings (for crates.io publishing)

### Version Numbering

We follow [Semantic Versioning](https://semver.org/):

- **MAJOR** version for incompatible API changes
- **MINOR** version for backwards-compatible functionality additions
- **PATCH** version for backwards-compatible bug fixes

### Release Steps

#### Option 1: Automated Release (Recommended)

1. **Update version numbers:**
   ```bash
   # Update version in all crates
   # Edit crates/synddb-client/Cargo.toml
   # Edit crates/synddb-sequencer/Cargo.toml
   # Edit crates/synddb-benchmark/Cargo.toml
   ```

2. **Commit version bump:**
   ```bash
   git add -A
   git commit -m "chore: Bump version to v0.2.0"
   git push origin main
   ```

3. **Create and push a tag:**
   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

4. **Workflow triggers automatically:**
   - Go to [Actions tab](https://github.com/Syndicate/SyndDB/actions)
   - Watch the "Release" workflow build all platforms
   - Check the "Releases" page for the new release

### What Happens During Release

The release workflow performs these steps:

1. **Multi-platform builds** (parallel):
   - Linux x86_64: Builds on `ubuntu-latest`
   - macOS x86_64: Builds on `macos-13`
   - macOS ARM64: Builds on `macos-14`
   - Windows x86_64: Builds on `windows-latest`

2. **Artifact preparation**:
   - Renames libraries with platform suffixes
   - Generates SHA256 checksums
   - Includes C header file (`synddb.h`)

3. **GitHub Release**:
   - Creates release with tag name
   - Uploads all platform libraries
   - Uploads checksums file
   - Adds comprehensive release notes

4. **crates.io publishing**:
   - Publishes `synddb-client` to crates.io
   - Only on tag pushes (not manual triggers)

## Workflow Details

The unified `build-ffi-libs.yml` workflow includes:

### Jobs

1. **build-libs** (matrix)
   - Builds all platforms in parallel
   - Conditional: Skips release-only platforms on dev builds
   - Uses `build_for: both` or `build_for: release` flags
   - Uploads artifacts with context-appropriate retention

2. **commit-dev-libs**
   - Runs only on `main` commits (not tags)
   - Downloads dev artifacts (Linux, macOS ARM64)
   - Commits to `crates/synddb-client/libs/` with `[skip ci]`

3. **create-release**
   - Runs only on tags
   - Downloads all artifacts (4 platforms)
   - Renames with platform suffixes
   - Generates checksums
   - Creates GitHub Release

4. **publish-crates**
   - Runs only on tags
   - Publishes to crates.io (if token configured)

## Continuous Integration

See `.github/workflows/rust-ci.yml` for testing and benchmarking:

1. **Tests**: `cargo test --all`
2. **Benchmarks**: Performance regression detection
3. **Code Quality**: Clippy, rustfmt (in `rust-checks.yml`)

FFI library building is handled separately by `build-ffi-libs.yml`

## Using Pre-Built Libraries

After a release, users can download pre-built libraries:

### Direct Download

```bash
# Download for your platform
curl -LO https://github.com/Syndicate/SyndDB/releases/download/v0.2.0/synddb_client-linux-x86_64.so
curl -LO https://github.com/Syndicate/SyndDB/releases/download/v0.2.0/synddb.h
```

### Verify Checksums

```bash
curl -LO https://github.com/Syndicate/SyndDB/releases/download/v0.2.0/SHA256SUMS
sha256sum -c SHA256SUMS
```

### Language-Specific Installation

**Python (ctypes):**
```python
# Download library for your platform
# Use examples/ffi/test.py as reference
```

**Node.js (koffi/ffi-napi):**
```bash
npm install
# Download library and place in project root
# Use examples/ffi/test_koffi.js as reference
```

**Go (cgo):**
```bash
# Download library
# Use bindings/go/ as reference
```

## Building Locally

If you need to build the FFI library yourself:

```bash
# Build for current platform
cargo build --package synddb-client --features ffi --release

# Output location
ls -lh target/release/libsynddb_client.*
```

## Troubleshooting

### Release Workflow Failed

1. Check Actions logs for specific error
2. Common issues:
   - Version already exists on crates.io (expected, won't fail)
   - Build failures on specific platforms
   - Missing secrets (CARGO_REGISTRY_TOKEN)

### Missing Platform Binary

If a platform binary is missing from the release:
1. Check if that platform's build job failed
2. Re-run the failed job or re-trigger the workflow
3. Build locally on that platform if needed

### crates.io Publish Failed

- Ensure CARGO_REGISTRY_TOKEN is set in repository secrets
- Check if version already published (idempotent, won't fail)
- Verify Cargo.toml has correct metadata

## Release Checklist

Before creating a release:

- [ ] All tests pass on `main`
- [ ] Benchmarks show no regressions
- [ ] Version numbers updated in Cargo.toml files
- [ ] CHANGELOG.md updated
- [ ] Examples tested and working
- [ ] Documentation up to date
- [ ] FFI header file (`synddb.h`) is current

After release:

- [ ] Verify all platform binaries present in GitHub Release
- [ ] Test downloads work
- [ ] Verify checksums match
- [ ] Check crates.io shows new version
- [ ] Announce release (if public)

## Future Improvements

Potential enhancements to the release process:

- [ ] Publish to npm (@synddb/client-native)
- [ ] Publish to PyPI (synddb-client)
- [ ] Automated changelog generation
- [ ] Automated version bumping
- [ ] Cross-compilation for more platforms (ARM Linux, etc.)
- [ ] Signed binaries
- [ ] Notarization for macOS binaries
