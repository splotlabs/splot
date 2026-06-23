## 1. Decoder Implementation

- [x] 1.1 Add a crate-private context-update config and context-commit wrapper around the derived-base/derived-sign ordinary nonzero pass.
- [x] 1.2 Source `culLevel` and `dcCategory` from the final quant-state summary and commit them through `TileCoeffContextState::update_after_coeffs`.
- [x] 1.3 Preserve the ordinary pass result for later dequant/reconstruction handoff and keep existing staged pass APIs intact.

## 2. Tests

- [x] 2.1 Add successful above/left level and DC context commit coverage.
- [x] 2.2 Add ordinary-pass failure coverage proving tile context lines are unchanged.
- [x] 2.3 Add invalid context-update geometry coverage proving no partial context-line mutation after the pass succeeds.

## 3. Tracking And Gates

- [x] 3.1 Add `DECODE-COEFF-NONZERO-CONTEXT-COMMIT` rows to implementation, decoder-support, conformance coverage, and roadmap docs.
- [x] 3.2 Regenerate feature/spec/support/decoder-coverage status docs.
- [x] 3.3 Run focused tests, OpenSpec validation, feature/support checks, and `cargo xtask ci`.
