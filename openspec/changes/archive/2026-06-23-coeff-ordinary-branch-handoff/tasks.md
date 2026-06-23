## 1. Tracking

- [x] 1.1 Add `DECODE-COEFF-ORDINARY-BRANCH-HANDOFF` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for `coeff-ordinary-branch-handoff`.

## 2. Implementation

- [x] 2.1 Add a crate-private ordinary branch handoff API that dispatches all-zero and nonzero coefficient branches.
- [x] 2.2 Route the minimal all-zero block-symbol trace through the new all-zero arm without changing output.
- [x] 2.3 Add focused tests for all-zero preservation, nonzero state-backed ordinary pass success, invalid nonzero-start preservation, and ordinary-pass failure state preservation.

## 3. Documentation And Verification

- [x] 3.1 Update the decoder roadmap and regenerate feature/status coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, and the Rust acceptance gate.
