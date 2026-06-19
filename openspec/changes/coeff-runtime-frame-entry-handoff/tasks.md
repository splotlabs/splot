## 1. Runtime Handoff

- [x] 1.1 Add `DECODE-COEFF-RUNTIME-FRAME-ENTRY-HANDOFF` to the implementation matrix.
- [x] 1.2 Route the minimal luma all-zero coefficient application through the frame-facts wrapper.
- [x] 1.3 Route the minimal V-plane all-zero coefficient application through the frame-facts wrapper.
- [x] 1.4 Preserve existing CDF rollback and output identity behavior.

## 2. Tests

- [x] 2.1 Add or update focused block-symbol tests for the wrapper entry path.
- [x] 2.2 Run minimal runtime hash/raw/Y4M tests proving output identity remains unchanged.
- [x] 2.3 Run focused tile-payload/block-symbol tests.

## 3. Tracking and Documentation

- [x] 3.1 Add the decoder-support row.
- [x] 3.2 Update decoder conformance coverage metadata.
- [x] 3.3 Update `docs/DECODER-ROADMAP.md`.
- [x] 3.4 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/DECODER-SUPPORT-STATUS.md`, and `docs/DECODER-SPEC-COVERAGE.md`.

## 4. Validation

- [x] 4.1 Run `openspec validate coeff-runtime-frame-entry-handoff --strict`.
- [x] 4.2 Run `openspec validate --all --no-interactive`.
- [x] 4.3 Run `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.4 Run `git diff --check`.
- [x] 4.5 Run `cargo xtask audit-scope --all --write-ledger`.
- [x] 4.6 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
