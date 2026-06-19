## 1. Q-Context Handoff

- [x] 1.1 Add a crate-private AV2 § 6.17.2 helper that derives `coeff_cdf_q_ctx` from `base_q_idx`.
- [x] 1.2 Add crate-private base-q input types for `DECODE-COEFF-CDF-Q-CONTEXT-HANDOFF`.
- [x] 1.3 Implement all-zero routing through the existing shared-facts path without requiring `base_q_idx`.
- [x] 1.4 Implement nonzero q-context derivation from `base_q_idx` before delegating to the existing shared-facts wrapper.

## 2. Tests

- [x] 2.1 Add focused tests for exact `base_q_idx` threshold boundaries and out-of-domain totality.
- [x] 2.2 Add focused tests proving all-zero base-q output matches the existing all-zero selector path.
- [x] 2.3 Add focused tests proving ordinary selected-branch behavior matches explicit shared-facts q-context delegation across all four q buckets.
- [x] 2.4 Add focused tests proving FSC selected-branch behavior matches explicit shared-facts q-context delegation across all four q buckets.
- [x] 2.5 Run focused `splot-decode` coefficient-loop tests.

## 3. Tracking and Documentation

- [x] 3.1 Add `DECODE-COEFF-CDF-Q-CONTEXT-HANDOFF` to `docs/IMPLEMENTATION-MATRIX.toml` with proof tests and commands.
- [x] 3.2 Add the decoder-support row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 3.3 Update `docs/DECODER-ROADMAP.md` and decoder conformance coverage metadata for the new partial handoff.
- [x] 3.4 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/DECODER-SUPPORT-STATUS.md`, and `docs/DECODER-SPEC-COVERAGE.md`.

## 4. Validation

- [x] 4.1 Run `openspec validate coeff-cdf-q-context-handoff --strict`.
- [x] 4.2 Run `openspec validate --all --no-interactive`.
- [x] 4.3 Run `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.4 Run `git diff --check`.
- [x] 4.5 Run `cargo xtask audit-scope --all --write-ledger`.
- [x] 4.6 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
