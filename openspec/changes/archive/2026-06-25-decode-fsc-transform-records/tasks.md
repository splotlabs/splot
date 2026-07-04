## 1. Runtime FSC Record Handoff

- [x] 1.1 Thread retained `fsc_mode` facts through the local decoder mission selectable-transform record path without widening decoded sample support.
- [x] 1.2 Derive the luma nonzero residual branch from AV2 §5.20.7.27 `useFsc` via the existing coefficient frame-facts wrapper.
- [x] 1.3 Preserve existing all-zero and non-FSC selectable transform-record behavior, including fail-closed diagnostics for unsupported branches.

## 2. Tests

- [x] 2.1 Add focused unit tests proving active luma `fsc_mode` selects the FSC coefficient handoff and records LR tx-skip facts.
- [x] 2.2 Add focused regression tests proving all-zero/non-FSC records do not validate or require FSC-only facts.
- [x] 2.3 Update the ignored local decoder mission CLI test expectation to the next structured unsupported-feature frontier.

## 3. Tracking and Proof

- [x] 3.1 Update `docs/IMPLEMENTATION-MATRIX.toml` and `docs/DECODER-SUPPORT-MATRIX.toml` for `DECODE-SELECTABLE-TRANSFORM-RECORDS`.
- [x] 3.2 Regenerate decoder support/status docs affected by the matrix updates.
- [x] 3.3 Run the focused Rust tests, the ignored local decoder mission probe, `openspec validate --all --no-interactive`, `cargo xtask feature-status`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, `cargo xtask check-fixtures`, `cargo xtask conformance`, and `cargo xtask ci`.
