## 1. Tracking

- [x] 1.1 Add `DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-CONTEXT` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for the tx-size-context ordinary branch handoff.

## 2. Implementation

- [x] 2.1 Derive `Tx_Size_Sqr[txSz]` and `Tx_Size_Sqr_Up[txSz]` in the ordinary branch `txSz` wrapper.
- [x] 2.2 Feed the derived `txSzCtx` into the ordinary base-context pass while preserving raw and adjusted dimension usage.
- [x] 2.3 Add focused tests for rectangular `txSzCtx` derivation, all-zero preservation, and invalid square-table fail-atomicity.

## 3. Documentation And Verification

- [x] 3.1 Update the decoder roadmap and regenerate feature/status coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, generated-table drift, focused tests, and the Rust acceptance gate.
