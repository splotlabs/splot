## 1. OpenSpec and Feature Tracking

- [x] 1.1 Validate the `coeff-eob-branch-handoff` OpenSpec artifacts.
- [x] 1.2 Add `DECODE-COEFF-EOB-BRANCH-HANDOFF` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.3 Add the corresponding decoder support and conformance-coverage rows.

## 2. Decode Helper

- [x] 2.1 Add a crate-private coefficient EOB branch handoff in `tile_payload/coeff_loop.rs`.
- [x] 2.2 Route all-zero branches to existing all-zero state application.
- [x] 2.3 Route nonzero branches to existing derived EOB symbol reading.
- [x] 2.4 Use the handoff in the minimal block-symbol trace's current all-zero coefficient paths.

## 3. Tests and Documentation

- [x] 3.1 Add focused tests for all-zero no-CDF/no-symbol consumption, nonzero state preservation, and invalid nonzero no-mutation behavior.
- [x] 3.2 Update decoder roadmap/status notes while preserving partial runtime support claims.
- [x] 3.3 Regenerate feature/status/support/coverage Markdown outputs.

## 4. Verification

- [x] 4.1 Run focused `splot-decode` coefficient-loop tests.
- [x] 4.2 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, decoder support, decoder conformance coverage, and OpenSpec validation.
- [x] 4.3 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
