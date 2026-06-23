## 1. Tracking

- [x] 1.1 Add `DECODE-COEFF-ORDINARY-BRANCH-SCAN-ORDER` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for the scan-order ordinary branch handoff.

## 2. Implementation

- [x] 2.1 Add a decode-local AV2 section 5.20.7.30 scan-order derivation helper for `TX_CLASS_2D`, `TX_CLASS_HORIZ`, and `TX_CLASS_VERT`.
- [x] 2.2 Remove caller-provided `scan` from the ordinary branch `txSz` wrapper and feed the derived scan order into the lower ordinary pass.
- [x] 2.3 Add focused tests for 2D, horizontal, and vertical scan derivation, all-zero preservation, and invalid scan-shape fail-atomicity.

## 3. Documentation And Verification

- [x] 3.1 Update the decoder roadmap and regenerate feature/status coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, focused tests, and the Rust acceptance gate.
