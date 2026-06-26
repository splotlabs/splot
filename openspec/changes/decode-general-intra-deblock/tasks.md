# Tasks

## Deblocking Orchestration

- [x] 1.1 Add `crates/splot-decode/src/runtime_minimal/deblock.rs` implementing
      the AV2 § 7.17.1 / § 7.17.2 edge traversal over the `splot-recon` per-edge
      primitives (`deblock_filter_choice`, `deblock_sample_filter`,
      `deblock_filter_max_width`, `deblock_adaptive_filter_strength`,
      `deblock_side_threshold_index`): derive the per-(plane, pass) § 7.17.6
      filter level, the § 7.17.5 (qThr, side) strengths over the § 9.2
      `Side_Thresholds` table, iterate the plane × pass × MI loop (4:2:0 chroma
      `rowStep`/`colStep`), and apply the § 7.17.4 filterSize / § 7.17.3
      max-width / § 7.17.7.2 choice / § 7.17.7.1 sample filter in place.
- [x] 1.2 Add the checked `CurrentFrameWorkspace::set_reconstructed_sample`
      single-sample writer to `splot-recon` for the in-place deblock write-back.
- [x] 1.3 Record each decoded leaf's deblocking geometry (luma/chroma transform
      sizes, base MI position) during the partition walk, and run the deblock
      pass after the walk and before `workspace.freeze()`.

## Route Gate

- [x] 2.1 Relax the general intra route gate to admit a deblock-active frame in
      the verified subset (`df_delta_q` all zero), keeping GDF/CDEF/CCSO/loop-
      restoration rejected.
- [x] 2.2 Reject a nonzero `df_delta_q` deblock-active frame and a 10-bit
      deblock-active frame (no oracle fixture pins them); a deblock-off frame
      runs the pass as a no-op at any bit depth.

## Tests And Tracking

- [x] 3.1 Add the `syn-2sb-deblock-intra-128x64-q100.ivf`,
      `syn-2sb-deblock-intra-128x64-q98.ivf`, and
      `syn-2sb-deblockwide-intra-128x64-q100.ivf` conformance fixtures and a
      positive decode test pinning the deblocked frame hash (raw md5
      `ca302adc8641007251c5947b3d5c73ba` for the q100 fixture).
- [x] 3.2 Confirm a deblock-off frame (apply == [false; 4]) stays byte-identical
      and the existing 8-bit and 10-bit corpus is unchanged.
- [x] 3.3 Add matrix, decoder-support, LOCAL-REFERENCE-EVIDENCE, and conformance
      manifest entries for `DECODE-GENERAL-INTRA-DEBLOCK`.
- [x] 3.4 Regenerate generated docs and run the required checks
      (`cargo xtask ci`, `conformance`, `check-fixtures`).
- [x] 3.5 Pin the admitted-but-not-natively-sample-changing paths (DEBLOCK-001):
      add the `syn-grid-deblock-intra-128x128-q100.ivf` multi-superblock-row
      fixture (raw md5 `1e4675e63da02a22431390e293e4c0ba`) exercising the y=64
      `sbEdge` iteration end-to-end, plus deterministic forced-`apply` unit tests
      for the luma-vertical (x=64) pass and the luma-horizontal y=64 `sbEdge`
      pass (with its negative-side max-width cap).
