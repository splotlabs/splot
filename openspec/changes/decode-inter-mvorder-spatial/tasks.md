# Tasks

## 1. Investigation (decide if this is a clean single brick)
- [x] 1.1 Confirm the honesty gap: every committed inter fixture propagates ONE
      identical MV (col 48), so the § 7.12.2 stack collapses and the per-neighbour
      ORDERING / DRL slot selection are exercised but not bit-exactly discriminated.
- [x] 1.2 Trace the § 7.12.2.6 ordered spatial scan for a 32x32 bottom-right leaf:
      step 7 left = `scan_point(bh4 - 1, -1)` BEFORE step 8 above =
      `scan_point(-1, bw4 - 1)`, so slot 0 = left, slot 1 = above.
- [x] 1.3 Confirm a 32x32 leaf is exactly 32x32 (`Block_Width` / `Block_Height`
      == 32, not > 32), so § 7.12.2.20 large-block MVP is inapplicable — the
      ordering can be pinned without implementing § 7.12.2.20 (the smaller brick).

## 2. Fixture (verify oracles first)
- [x] 2.1 Generate `syn-2frame-inter-mvorder-64x64.ivf` from a project-owned
      synthetic Y4M (64x64: four flat 32x32 luma quadrants 100/150/60/200 + flat
      chroma; frame 1 = each quadrant shifted by a DIFFERENT amount — TL +8, TR -4,
      BL +4, BR -4 luma samples, edge-clamped) at `--qp 80 --sb-size 64
      --min-partition-size 32 --max-partition-size 32 --enable-rect-partitions=1`
      with broad decode tools disabled. Confirm the encode is deterministic
      (byte-identical re-encode).
- [x] 2.2 Confirm avmdec `--rawvideo --i420` == dav2d `--demuxer ivf`
      byte-for-byte (md5 `284e1450b42180f02de7415ab0367bfe`, 12288 bytes).
- [x] 2.3 Confirm via temporary splot instrumentation that the four 32x32 leaves
      carry DISTINCT MVs: block 0 NEWMV col 64, block 1 NEWMV col -32, block 2
      NEWMV col 32, block 3 NEARMV RefMvIdx 1 over a stack
      `[col 32 (left), col -32 (above), col 64 (corner), col 0 (global)]`
      reconstructing col -32 (the above neighbour). Remove the instrumentation.
- [x] 2.4 Register in the conformance manifest + reciprocal
      LOCAL-REFERENCE-EVIDENCE entry.

## 3. Pin the ordering (no decoder change)
- [x] 3.1 Confirm `find_mv_stack` decodes the distinct-MV fixture bit-exact with no
      code change (the spatial scan order and search-stack dedupe were already
      correct).
- [x] 3.2 Prove falsifiability locally: temporarily swap scan steps 7 and 8 and
      confirm both the `find_mv_stack` unit test (slot 0 becomes the above MV) and
      the bit-exact decode-hash test FAIL; then revert.
- [x] 3.3 Update the now-inaccurate `find_mv_stack.rs` comments: the ordering is
      PROVEN by a distinct-MV fixture; § 7.12.2.20 large-block is inapplicable to
      the 32x32 leaves (so the ordering is pinned without it) and stays deferred
      for the > 32x32 leaves.

## 4. Verify + gate
- [x] 4.1 `splot decode syn-2frame-inter-mvorder-64x64.ivf --output-format raw` ==
      oracle md5 byte-for-byte.
- [x] 4.2 Add the `find_mv_stack` distinct-MV ordering unit test + the per-frame
      hash decode test + the CLI raw-output round-trip test.
- [x] 4.3 All existing inter (zero-MV, sub-pel, residual, mvstack, SB-row, grid) +
      general-intra fixtures byte-identical (no regression).
- [x] 4.4 `cargo xtask ci` passes; `openspec validate --all` clean.

## 5. Deferred (out of scope, gated absent before output)
- [ ] 5.1 The § 7.12.2.20 large-block (> 32x32) extra MVP combinations
      (`insert_mvp_candidate`) — observable only with a distinct-MV > 32x32 leaf
      whose RefMvIdx selects a mixed-only slot.
- [ ] 5.2 A multi-superblock skip == 0 residual (per-block transform sizes).
- [ ] 5.3 The deferred § 7.12.2 candidates inherited from
      `DECODE-INTER-MVSTACK-SPATIAL` (temporal, compound, warp, ref-MV bank,
      derived-SMVP, DRL reorder, scan-col wider reach).
