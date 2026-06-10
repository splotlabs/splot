# Testing

## Strategy (in priority order)

1. **Parser unit tests** — LEB128, AV2 OBU header, and Annex B envelopes, with
   positive, negative, and EOF cases. Implemented in each `splot-core` module.
2. **Property / fuzz tests** — the parsers must never panic on arbitrary input.
   Implemented as `*_never_panic(s)` proptests across the `splot-core` parser
   modules; the `cargo fuzz` target `parse_obu` needs a nightly toolchain and
   runs as a blocking 60s smoke in PR CI.
3. **CLI integration tests** — `crates/splot-cli/tests/cli.rs` runs the `splot`
   binary against the fixtures in `tests/fixtures/` (exit codes, `--json`,
   `inspect`). Implemented; snapshot tests for `inspect` output are planned
   (`insta`).
4. **Conformance vectors** — from AOMedia. Planned, once vectors are available
   (see [CONFORMANCE.md](./CONFORMANCE.md)).
5. **Differential testing against AVM** — the reference software is the oracle.
   Planned (directions and harness plan in [CONFORMANCE.md](./CONFORMANCE.md)).

## Commands

```bash
cargo test --workspace --all-targets --locked   # unit, property, and CLI integration tests (no doctests)
cargo test --doc --workspace --locked           # doctests (not covered by --all-targets)
cargo xtask ci
cargo xtask coverage            # local HTML coverage report (cargo-llvm-cov, run-if-present)

# Fuzzing needs a NIGHTLY toolchain (cargo-fuzz uses AddressSanitizer + coverage,
# which are nightly-only). On stable, the per-module `*_never_panic(s)` proptests
# exercise the same never-panic invariant with bounded random inputs.
cargo xtask fuzz [--time <secs>]    # local fuzz smoke (nightly + cargo-fuzz, run-if-present), default 30s
cargo install cargo-fuzz --locked
cargo +nightly fuzz run parse_obu

cargo xtask conformance         # stub for AVM differential testing
```

## Conventions

- Every parser change adds the relevant positive/negative/EOF cases.
- Tests may use `unwrap`/`expect` only inside `#[cfg(test)]` modules annotated with
  `#[allow(clippy::unwrap_used, clippy::expect_used)]`; production library code must
  not.
- **Record proof in the matrix.** When a feature's stage becomes `done`, record the
  test module/path, the reproducible command, the fixture/vector, and/or the
  diagnostic id in that row's `[feature.proof]` in
  [IMPLEMENTATION-MATRIX.toml](./IMPLEMENTATION-MATRIX.toml). `cargo xtask
  check-feature-status` rejects a `done` code stage with no proof; `cargo xtask
  spec-coverage` lists rows still missing proof.
