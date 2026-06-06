# Testing

## Strategy (in priority order)

1. **Parser unit tests** — LEB128, AV2 OBU header, and Annex B envelopes, with
   positive, negative, and EOF cases. (Implemented in each `splot-core` module.)
2. **Property / fuzz tests** — the parsers must never panic on arbitrary input.
   (`proptest` in `splot-core::annexb`; `cargo fuzz` target `parse_obu`.)
3. **Snapshot tests** — for `splot inspect` output. (Planned: `insta`.)
4. **Conformance vectors** — from AOMedia, once available. (Planned:
   `cargo xtask fetch-vectors`.)
5. **Differential testing against AVM** — the reference software is the oracle:
   - `avm encode` → `splot validate`
   - future: `splot encode` → `avm decode`
   (Planned: `cargo xtask conformance`.)

## Commands

```bash
cargo test --workspace --all-targets --locked
cargo xtask ci
cargo fuzz run parse_obu        # requires `cargo install cargo-fuzz --locked`
cargo xtask conformance         # stub for AVM differential testing
```

## Conventions

- Every parser change adds the relevant positive/negative/EOF cases.
- Tests may use `unwrap`/`expect` only inside `#[cfg(test)]` modules annotated with
  `#[allow(clippy::unwrap_used, clippy::expect_used)]`; production library code must
  not.
