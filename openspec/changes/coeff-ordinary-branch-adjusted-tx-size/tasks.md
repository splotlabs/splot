## 1. Tracking

- [x] 1.1 Add `DECODE-COEFF-ORDINARY-BRANCH-ADJUSTED-TX-SIZE` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for the adjusted-size ordinary branch handoff.

## 2. Implementation

- [x] 2.1 Derive `Adjusted_Tx_Size[txSz]` in the ordinary branch `txSz`-dimensions wrapper.
- [x] 2.2 Feed adjusted width, height, and width-log2 dimensions into the ordinary base-context pass while preserving raw dimensions for EOB and block geometry.
- [x] 2.3 Add focused tests for adjusted 64-sample-side behavior, all-zero preservation, and invalid adjusted-size fail-atomicity.

## 3. Documentation And Verification

- [x] 3.1 Update the decoder roadmap and regenerate feature/status coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, generated-table drift, focused tests, and the Rust acceptance gate.
