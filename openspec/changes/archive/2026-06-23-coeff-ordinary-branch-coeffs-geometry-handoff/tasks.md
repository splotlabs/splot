## 1. Tracking

- [x] 1.1 Add `DECODE-COEFF-ORDINARY-BRANCH-COEFFS-GEOMETRY-HANDOFF` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for `coeff-ordinary-branch-coeffs-geometry-handoff`.

## 2. Implementation

- [x] 2.1 Add a crate-private ordinary branch input wrapper that derives block geometry from `coeffs()` geometry facts.
- [x] 2.2 Delegate the wrapper to the existing geometry handoff without changing all-zero behavior.
- [x] 2.3 Add focused tests for nonzero geometry equivalence and all-zero preservation.

## 3. Documentation And Verification

- [x] 3.1 Update the decoder roadmap and regenerate feature/status coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, and the Rust acceptance gate.
