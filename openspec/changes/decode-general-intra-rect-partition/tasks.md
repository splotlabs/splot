## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-RECT-PARTITION` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for `general-intra-rect-partition`.
- [x] 1.3 Add the `syn-hrect-intra-64x64-q120.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Generalise `decode_general_intra_plane_coeffs` context spans to read `Tx_Width[txSz] >> 2` and `Tx_Height[txSz] >> 2` independently from the § 9.2 conversion tables.
- [x] 2.2 Add `reconstruct_general_intra_block_rect` / `reconstruct_general_intra_block_rect_into` for the rectangular DC reconstruction (separate `log2_width` / `log2_height`, the § 7.15.4.1 √2 rescale path).
- [x] 2.3 Add the rectangular leaf dispatch in `decode_one_general_intra_rect_block`, gated to DC luma + DC chroma; derive the rectangular `txSz` from the block width/height log2 via the conversion tables; reject non-DC rectangular modes.

## 3. Verification

- [x] 3.1 Confirm via temporary instrumentation that the fixture's superblock SPLITs via PARTITION_HORZ into two rectangular 64x32 DC_PRED leaves (both `n4w == 16`, `n4h == 8`, DC luma).
- [x] 3.2 Confirm `splot decode --output-format raw` equals avmdec `--rawvideo --i420` AND dav2d `--demuxer ivf` byte-for-byte and pin the frame hash in a decode test.
- [x] 3.3 Confirm all existing general intra fixtures still decode bit-exact.
- [ ] 3.4 (deferred) Add negative conformance vectors for the by-construction rejects (non-DC rectangular luma `general_intra_rect_non_dc_luma`, non-DC rectangular chroma `general_intra_rect_non_dc_chroma`, and non-64x32 rectangular geometry `general_intra_rect_unverified_geometry`). These rejects are fail-safe (returned before any coefficient read or sample write) but are not yet individually fixture-backed.

## 4. Documentation And Verification

- [x] 4.1 Regenerate feature/status/coverage docs.
- [x] 4.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, reference evidence, and the Rust acceptance gate.
