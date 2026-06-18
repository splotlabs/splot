## 1. Implementation

- [x] 1.1 Add Feature ID `DECODE-COEFF-SCAN-WALK` to the implementation matrix.
- [x] 1.2 Add the crate-private scan-walk helper, typed inputs/results/errors,
  and module wiring in `splot-decode`.
- [x] 1.3 Add focused tests for reverse scan order, EOB length rejection,
  out-of-range scan-position rejection, and no mutation/consumption behavior.

## 2. Documentation and Coverage

- [x] 2.1 Add the decoder support matrix row and decoder roadmap note for the
  narrow scan-walk boundary.
- [x] 2.2 Add decoder conformance coverage metadata for `DECODE-COEFF-SCAN-WALK`.
- [x] 2.3 Regenerate feature, spec, decoder support, and decoder conformance
  status documents.

## 3. Verification

- [x] 3.1 Run focused `splot-decode` tests for coefficient-loop behavior.
- [x] 3.2 Run `openspec validate coeff-scan-walk --strict`.
- [x] 3.3 Run feature/support/conformance checks and full
  `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
