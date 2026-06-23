## 1. FSC Segment Handoff

- [x] 1.1 Add crate-private FSC branch segment-extents input/result wiring that derives `segEob` from scan length and delegates to the explicit FSC branch.
- [x] 1.2 Preserve existing all-zero, non-luma, and invalid-scan mutation boundaries.

## 2. Tests

- [x] 2.1 Add focused positive coverage proving the derived-`segEob` wrapper matches the explicit staged branch.
- [x] 2.2 Add failure coverage for all-zero, non-luma, and short-scan behavior with symbol/CDF/context mutation boundaries.

## 3. Tracking And Validation

- [x] 3.1 Add `DECODE-COEFF-FSC-BRANCH-SEG-EOB-HANDOFF` rows to the implementation matrix, decoder support matrix, decoder conformance coverage, and roadmap.
- [x] 3.2 Regenerate generated status documents and run OpenSpec, feature/support/conformance, focused decode tests, `git diff --check`, and full `cargo xtask ci`.
