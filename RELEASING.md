# Release Process

This document describes how to create a new release of SyndDB.

## Overview

SyndDB uses automated GitHub Actions workflows to build multi-platform binaries and publish releases. The release process includes:

1. Building native FFI libraries for all supported platforms (Linux, macOS, Windows)
2. Creating GitHub releases with downloadable artifacts
3. Publishing to crates.io (Rust package registry)

## Supported Platforms

The release workflow automatically builds native libraries for:

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

#### Option 2: Manual Trigger

You can manually trigger a release workflow without creating a tag:

1. Go to [Actions](https://github.com/Syndicate/SyndDB/actions)
2. Select "Release" workflow
3. Click "Run workflow"
4. Enter version (e.g., `v0.2.0`)
5. Click "Run workflow"

### What Happens During Release

The release workflow performs these steps:

1. **Multi-platform builds** (parallel):
   - Linux: Builds on `ubuntu-latest`
   - macOS x86_64: Builds on `macos-13`
   - macOS ARM64: Builds on `macos-14`
   - Windows: Builds on `windows-latest`

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

## Continuous Integration

Every PR and push to `main` automatically:

1. **Runs tests** (`cargo test --all`)
2. **Builds FFI library** (Linux x86_64 only)
3. **Uploads artifacts** (available for 7 days)
4. **Runs benchmarks** with regression detection

Access build artifacts:
- Go to Actions → Select workflow run → Scroll to "Artifacts"
- Download `libsynddb_client-linux-x86_64.zip`

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
