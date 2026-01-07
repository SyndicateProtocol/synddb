# RISC Zero TODO

Optional enhancements identified from RISC Zero documentation research.

## Guest Code Optimizations

Add to `crates/synddb-bootstrap/risc0/program/Cargo.toml`:

```toml
[profile.release]
lto = "thin"           # Often faster than "fat" for RISC Zero
opt-level = 2          # Sometimes faster than opt-level 3
codegen-units = 1      # Better optimization at cost of compile time
```

Additional guest code tips:
- Use `BTreeMap` instead of `HashMap` (deterministic, no hasher overhead)
- Leverage SHA-256 precompile (68 cycles per 64-byte block vs software impl)
- Avoid floating-point operations (60-140 cycles vs ~1 for integers)
- Align data structures to 4-byte boundaries for optimal memory access

## Memory Configuration for Constrained Environments

For systems with <16GB RAM, configure segment limits:

```rust
let env = ExecutorEnv::builder()
    .segment_limit_po2(18)  // Smaller segments for lower memory usage
    .write(&input)?
    .build()?;
```

## CI Caching

Cache the RISC Zero toolchain directory to speed up CI:

```yaml
- name: Cache RISC Zero toolchain
  uses: actions/cache@v4
  with:
    path: ~/.risc0
    key: risc0-${{ runner.os }}-3.0.4
```

## Reproducible Builds

For production ImageID verification, use Docker-based reproducible builds:

```bash
cargo risczero build --manifest-path crates/synddb-bootstrap/risc0/program/Cargo.toml
```

This ensures the ImageID is reproducible across different build environments.
Output: `./target/riscv-guest/riscv32im-risc0-zkvm-elf/docker/`

## Execution Statistics in CI

Add to CI for visibility into cycle counts and segment information:

```yaml
env:
  RUST_LOG: "[executor]=info"
```

## Environment Variables Reference

| Variable | Purpose |
|----------|---------|
| `RISC0_DEV_MODE=true` | Skip proof generation during development |
| `RISC0_PPROF_OUT=profile.pb` | Generate profiling data for guest code |
| `RISC0_PPROF_ENABLE_INLINE_FUNCTIONS=yes` | Enable inline function tracking in profiles |
| `RUST_LOG=[executor]=info` | Get executor statistics (cycle counts, segments) |
| `RUST_LOG=risc0_zkp=debug` | Debug ZKP module |

## References

- [RISC Zero Installation](https://dev.risczero.com/api/zkvm/install)
- [Local Proving Guide](https://dev.risczero.com/api/generating-proofs/local-proving)
- [Dev Mode Documentation](https://dev.risczero.com/api/generating-proofs/dev-mode)
- [Guest Optimization Guide](https://dev.risczero.com/api/zkvm/optimization)
- [Verifier Contracts](https://dev.risczero.com/api/blockchain-integration/contracts/verifier)
