## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-BLOCK-DECODED` to the implementation matrix.
- [x] 1.2 Add decoder support and conformance coverage rows for `general-intra-block-decoded`.
- [x] 1.3 Add the `syn-shsplit-intra-64x64-q80.ivf` and `syn-svsplit-intra-64x64-q140.ivf` fixtures, conformance manifest entries, and reciprocal LOCAL-REFERENCE-EVIDENCE entries.

## 2. Implementation

- [x] 2.1 Add the crate-private `TileBlockDecodedState` (§ 5.20.2.3 `clear_block_decoded_flags`, § 5.20.7.25 `count_top_right_avail` / `count_bottom_left_avail`, § 5.20.4 per-block set) with unit tests.
- [x] 2.2 Wire the grid into `decode_general_intra_partition_tree`: create, clear per superblock, pass read-only to the leaf, and mark each decoded transform block's plane 4x4 units after the leaf.
- [x] 2.3 Derive the luma § 7.13.2.1 `num4AboveRight` from `count_top_right_avail` over the real grid and lift the `general_intra_multiblock_non_dc_subblock` reject for the verified SMOOTH_H luma SPLIT sub-block subset; keep SMOOTH_V and SMOOTH chroma sub-blocks rejecting.

## 3. Verification

- [x] 3.1 Confirm via temporary instrumentation that the fixture's 64x64 superblock SPLITs into four 32x32 squares and the bottom-left 32x32 codes SMOOTH_H, and that its § 7.13.2.1 above-right sentinel is the real decoded top-right sibling (210), not the edge clamp (~50).
- [x] 3.2 Confirm `splot decode --output-format raw` equals avmdec `--rawvideo --i420` AND dav2d `--demuxer ivf` byte-for-byte and pin the frame hash in a decode test.
- [x] 3.3 Confirm all existing general intra fixtures still decode bit-exact and a still-unsupported case (SMOOTH chroma SPLIT sub-block) rejects with `decode/unsupported-feature`.

## 4. Documentation And Verification

- [x] 4.1 Regenerate feature/status/coverage docs.
- [x] 4.2 Validate OpenSpec, feature tracking, decoder support, decoder conformance coverage, reference evidence, and the Rust acceptance gate.
