# Tasks

## Matrix and docs

- [x] Update `docs/IMPLEMENTATION-MATRIX.toml` `AV2-A-PROFILES` (`types=done`, notes,
      `openspec_change`).
- [x] Regenerate `docs/FEATURE-STATUS.md`.

## Implementation

- [x] Convert `ProfileIdc` to an enum over Annex A.2 Table A.1 (variants in `seq_profile_idc`
      order so `Ord` matches the raw value), keeping `from_bits`/`get`; add
      `is_reserved`/`is_configurable`.
- [x] Switch MSDO `multistream_profile_idc`, OPS `ops_seq_profile_idc`, LCR
      `lcr_seq_profile_idc` fields from `u8` to `ProfileIdc`.
- [x] Update validator consumers to extract `.get()` at the boundary (no behavior change).

## Tests and proof

- [x] `ProfileIdc` unit tests: round-trip every 5-bit value, Table A.1 classification,
      `Ord` matches raw-value order.
- [x] Full validator/core suites pass unchanged (behavior-preserving).

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
