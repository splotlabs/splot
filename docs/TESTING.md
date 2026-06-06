# Testing

## Strategy (in priority order)

1. **Parser unit tests** — LEB128, AV2 OBU header, and Annex B envelopes, with
   positive, negative, and EOF cases. (Implemented in each `splot-core` module.)
2. **Property / fuzz tests** — the parsers must never panic on arbitrary input.
   (`proptest` in `splot-core::annexb`, runs on stable; the `cargo fuzz` target
   `parse_obu` needs a nightly toolchain.)
3. **CLI integration tests** — `crates/splot-cli/tests/cli.rs` runs the `splot`
   binary against the fixtures in `tests/fixtures/` (exit codes, `--json`, `inspect`).
   Snapshot tests for `inspect` output are planned (`insta`).
4. **Conformance vectors** — from AOMedia, once available. (Planned:
   `cargo xtask fetch-vectors`.)
5. **Differential testing against AVM** — the reference software is the oracle:
   - `avm encode` → `splot validate`
   - future: `splot encode` → `avm decode`
   (Planned: `cargo xtask conformance`.)

## Commands

```bash
cargo test --workspace --all-targets --locked   # unit, property, and CLI integration tests
cargo xtask ci

# Fuzzing needs a NIGHTLY toolchain (cargo-fuzz uses AddressSanitizer + coverage,
# which are nightly-only). On stable, the `parsers_never_panic` proptest covers the
# same "never panics" invariant.
cargo install cargo-fuzz --locked
cargo +nightly fuzz run parse_obu

cargo xtask conformance         # stub for AVM differential testing
```

## Conventions

- Every parser change adds the relevant positive/negative/EOF cases.
- Tests may use `unwrap`/`expect` only inside `#[cfg(test)]` modules annotated with
  `#[allow(clippy::unwrap_used, clippy::expect_used)]`; production library code must
  not.
