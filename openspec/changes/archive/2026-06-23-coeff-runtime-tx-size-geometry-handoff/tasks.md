## 1. Runtime Handoff

- [x] 1.1 Add `DECODE-COEFF-RUNTIME-TX-SIZE-GEOMETRY-HANDOFF` to the implementation matrix.
- [x] 1.2 Replace local minimal-runtime `TX_64X64` and `TX_16X16` constants with generated-table geometry resolution.
- [x] 1.3 Preserve existing all-zero coefficient frame-entry behavior and CDF rollback.

## 2. Tests

- [x] 2.1 Add focused tests for luma and V geometry-to-`txSz` resolution through generated tables.
- [x] 2.2 Add or update a focused rejection test proving unsupported geometry consumes no CDF or symbol state.
- [x] 2.3 Run minimal runtime hash/raw/Y4M tests proving output identity remains unchanged.
- [x] 2.4 Run focused tile-payload/block-symbol tests.

## 3. Tracking and Documentation

- [x] 3.1 Add the decoder-support row.
- [x] 3.2 Update decoder conformance coverage metadata.
- [x] 3.3 Update `docs/DECODER-ROADMAP.md`.
- [x] 3.4 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/DECODER-SUPPORT-STATUS.md`, and `docs/DECODER-SPEC-COVERAGE.md`.

## 4. Validation

- [x] 4.1 Run `openspec validate coeff-runtime-tx-size-geometry-handoff --strict`.
- [x] 4.2 Run `openspec validate --all --no-interactive`.
- [x] 4.3 Run `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.4 Run `git diff --check`.
- [x] 4.5 Run `cargo xtask audit-scope --all --write-ledger`.
- [x] 4.6 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
