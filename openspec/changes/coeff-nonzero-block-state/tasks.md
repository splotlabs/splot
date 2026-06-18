## 1. OpenSpec and Feature Tracking

- [x] 1.1 Validate the `coeff-nonzero-block-state` OpenSpec artifacts.
- [x] 1.2 Add `DECODE-COEFF-NONZERO-BLOCK-STATE` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.3 Add the corresponding decoder support and conformance-coverage rows.

## 2. Decode Helper

- [x] 2.1 Move the EOB branch handoff code into a child module without changing behavior.
- [x] 2.2 Add nonzero coefficient block-start input/result types.
- [x] 2.3 Allocate zeroed local transform coefficient state before nonzero EOB reads.
- [x] 2.4 Update the branch handoff nonzero arm to return the block-start shell.

## 3. Tests and Documentation

- [x] 3.1 Add focused tests for nonzero block allocation and invalid-geometry no-consumption behavior.
- [x] 3.2 Update existing branch-handoff tests for the enriched nonzero result.
- [x] 3.3 Update decoder roadmap/status notes while preserving partial runtime support claims.
- [x] 3.4 Regenerate feature/status/support/coverage Markdown outputs.

## 4. Verification

- [x] 4.1 Run focused `splot-decode` coefficient-loop tests.
- [x] 4.2 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, decoder support, decoder conformance coverage, and OpenSpec validation.
- [x] 4.3 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
