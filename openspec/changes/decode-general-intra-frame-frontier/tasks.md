## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-FRAME-FRONTIER` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for `general-intra-frame-frontier`.

## 2. Fixture And Evidence

- [x] 2.1 Commit `syn-flat-intra-64x64-q80.ivf` and register it in the conformance manifest.
- [x] 2.2 Record the avmdec/dav2d raw-output agreement in `docs/LOCAL-REFERENCE-EVIDENCE.toml`.

## 3. Implementation

- [x] 3.1 Add `is_general_minimal_intra` and `route_general_minimal_intra` predicates that accept a minimal-tool intra key frame off the frozen `base_q_idx == 255` tier.
- [x] 3.2 Add `decode_general_minimal_intra_frame`, deriving the single tile work unit and running the real AV2 § 5.20.3.1 root partition traversal before returning a structured `general_intra_block_decode_unimplemented` diagnostic.
- [x] 3.3 Add CLI tests for the general path reaching the partition frontier and for the frozen-hash regression guard.

## 4. Documentation And Verification

- [x] 4.1 Update the decoder roadmap and regenerate feature/status/coverage docs.
- [x] 4.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, and the Rust acceptance gate.
