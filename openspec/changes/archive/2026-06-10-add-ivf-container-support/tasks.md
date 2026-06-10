## 1. Core Container Support

- [x] 1.1 Add an `AV2-IVF-CONTAINER` implementation-matrix row and docs mapping notes.
- [x] 1.2 Implement panic-free IVF header/frame parsing and writer helpers in `splot-core`.
- [x] 1.3 Add an input-format parser that auto-detects IVF vs raw Annex B and preserves original byte offsets.
- [x] 1.4 Add positive, malformed, EOF, and no-panic tests for IVF and input-format parsing.

## 2. Validator and CLI Integration

- [x] 2.1 Update `splot-validate` to validate container-aware inputs and emit stable `ivf/*` diagnostics.
- [x] 2.2 Update `splot inspect` and `splot validate` documentation/help text for IVF and raw Annex B inputs.
- [x] 2.3 Add CLI tests for IVF validate/inspect and malformed IVF diagnostics.

## 3. Documentation

- [x] 3.1 Update `README.md` and project docs to describe raw Annex B plus IVF support.
- [x] 3.2 Update `docs/VALIDATOR-DIAGNOSTICS.md` for the new `ivf/` namespace.
- [x] 3.3 Update generated feature status/spec coverage files after matrix changes.

## 4. Verification

- [x] 4.1 Run targeted parser/validator/CLI tests for IVF support.
- [x] 4.2 Run `cargo xtask feature-status` and `cargo xtask check-feature-status`.
- [x] 4.3 Run `cargo xtask ci`.
