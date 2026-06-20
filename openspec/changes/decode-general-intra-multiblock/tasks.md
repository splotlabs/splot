## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-MULTIBLOCK` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for `general-intra-multiblock`.
- [x] 1.3 Add the `syn-quad-intra-64x64-q80.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Add `decode_general_intra_partition_tree` walking the full §5.20.3.1 tree with a per-leaf callback and MI-size context.
- [x] 2.2 Add `decode_general_intra_plane_coeffs` deriving §8.3.2 txb_skip context from the persistent neighbour lines and threading one context across blocks.
- [x] 2.3 Reconstruct each leaf into a persistent workspace in decode order (`reconstruct_general_intra_block_into`), DC-predicting from reconstructed neighbours.
- [x] 2.4 Unify the single-block path through the driver, validate §8.2.4 exit_symbol(), and gate to DC_PRED square blocks.

## 3. Documentation And Verification

- [x] 3.1 Update the decoder roadmap and regenerate feature/status/coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, reference evidence, and the Rust acceptance gate.
