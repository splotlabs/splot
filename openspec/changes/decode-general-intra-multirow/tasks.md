## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-MULTIROW` to the implementation matrix.
- [x] 1.2 Add the `general-intra-multirow` decoder support row.
- [x] 1.3 Add the `syn-uniform-intra-128x128-q80.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Relax `is_general_minimal_intra` to accept width and height both positive multiples of 64 (a grid of 64x64 superblocks).

## 3. Documentation And Verification

- [x] 3.1 Regenerate feature/status/support docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, and the Rust acceptance gate.
