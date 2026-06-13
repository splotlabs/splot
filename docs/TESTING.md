# Testing

## Strategy (in priority order)

1. **Parser unit tests** — LEB128, AV2 OBU header, Annex B envelopes, and IVF
   container records, with positive, negative, and EOF cases. Implemented in each
   `splot-core` module.
2. **Property / fuzz tests** — the parsers and the validator must never panic on
   arbitrary input. Implemented as `*_never_panic(s)` tests across the
   `splot-core` parser modules and `crates/splot-validate/tests/validator_never_panics.rs`
   (mostly proptests, plus a few exhaustive-truncation unit tests). Four `cargo
   fuzz` targets cover every public byte-consuming entry point and need a nightly
   toolchain; they run as a blocking per-target smoke (~45s each) in PR CI:
   - `parse_obu` — `read_leb128`, `read_obu_header`, `parse_annex_b_obus`.
   - `parse_ivf` — `is_ivf`, `parse_ivf_header`, `parse_ivf_partial`.
   - `parse_bitstream` — `parse_bitstream_partial` (container auto-detect +
     Annex-B/IVF envelope parsing; OBU payload parsers are reached via
     `validate_bytes`, not this target).
   - `validate_bytes` — `Validator::validate_bytes_with_options` (the
     highest-coverage target: transitively reaches every OBU payload parser, both
     container formats, and every validator check).
3. **CLI integration tests** — `crates/splot-cli/tests/cli.rs` runs the `splot`
   binary against the fixtures in `tests/fixtures/` and generated temporary IVF
   inputs (exit codes, `--json`, `inspect`). Implemented; snapshot tests for
   `inspect` output are planned (`insta`).
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
# which are nightly-only). On stable, the per-module `*_never_panic(s)` tests and
# the splot-validate `validator_never_panics` proptest exercise the same
# never-panic invariant with bounded random inputs.
cargo xtask fuzz [--time <secs>]    # local fuzz smoke over every target (nightly + cargo-fuzz, run-if-present), default 30s each
cargo install cargo-fuzz --locked
cargo +nightly fuzz list            # parse_obu, parse_ivf, parse_bitstream, validate_bytes
cargo +nightly fuzz run parse_obu   # run a single target (swap the name for any of the four)

cargo xtask conformance         # run the committed conformance corpus (no AVM)
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
