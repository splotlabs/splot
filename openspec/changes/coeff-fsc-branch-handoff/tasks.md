## 1. FSC Branch Handoff

- [x] 1.1 Add crate-private FSC branch input/result/error types and a wrapper that rejects non-luma routing before EOB consumption.
- [x] 1.2 Compose nonzero EOB start, checked FSC scan walk, FSC level pass, and FSC quant/context commit without wiring runtime `coeffs()`.

## 2. Tests

- [x] 2.1 Add focused tests proving successful FSC branch output and context writes match the explicit staged pipeline.
- [x] 2.2 Add failure tests for all-zero routing, invalid scan/`segEob`, and chroma routing with mutation boundaries.

## 3. Tracking And Validation

- [x] 3.1 Add `DECODE-COEFF-FSC-BRANCH-HANDOFF` rows to the implementation matrix, decoder support matrix, decoder conformance coverage, and roadmap.
- [x] 3.2 Regenerate generated status documents and run OpenSpec, feature/support/conformance, focused decode tests, `git diff --check`, and full `cargo xtask ci`.
