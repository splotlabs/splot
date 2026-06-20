## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-BLOCK-MODES` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for `general-intra-block-modes`.

## 2. Implementation

- [x] 2.1 Add `decode_general_intra_block_modes` decoding the §5.20.5.3 mode symbols in spec order (`y_mode_set`, `y_mode_index` → typed `YMode`, `uv_mode` + escape).
- [x] 2.2 Wire it into `decode_general_minimal_intra_frame` after the partition frontier, reporting the residual step as unsupported.
- [x] 2.3 Add a unit test (DC luma + chroma mode in spec order) and update the CLI test for the new residual diagnostic.

## 3. Documentation And Verification

- [x] 3.1 Update the decoder roadmap and regenerate feature/status/coverage docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, and the Rust acceptance gate.
