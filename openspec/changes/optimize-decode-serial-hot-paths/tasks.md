# Tasks: optimize-decode-serial-hot-paths

- [x] 1. Cache `SPLOT_TRACE_*` env gates once per process and fast-gate the
      per-symbol trace hook; `#[inline]` the shared § 4.8 math helpers.
- [x] 2. Reuse a per-thread selectable-tx grid across coding blocks with a
      record-driven reset (key-frame and inter-frame walks).
- [x] 3. Derive LR unit membership per axis; coalesce LR source blocks into
      row runs; row-slice Wiener NS luma kernel with hoisted validation;
      chroma luma-downsample memoization; per-classification qval hoist;
      lazy filter-stage plane snapshots.
- [x] 4. CDEF per-block interior kernel; deblock stack-array lines with a
      per-pass strength memo; CCSO offset LUT over borrowed rows.
- [x] 5. Coefficient path: drop per-TU clones, preflight re-walks, and quant
      copies; per-thread residual scratch; inverse-transform kernel-once
      dispatch with zero skips and shift-algebra rounding.
- [x] 6. Motion compensation: strided borrowed `ReferencePlaneView` and the
      unscaled-step contiguous-row subpel fast path.
- [x] 7. Batch raw serialization and hash digest updates per row.
- [x] 8. Band both deblock passes on the owned pool (row bands for vertical
      edges, column bands for horizontal edges) behind the callee-side
      guard; flatten CfL plane reads; bucket LR coalescing per row; prove
      `write_rect` row bounds once per rectangle.
- [x] 9. Re-measure the phase table; confirm bit-exact raw sha256; run
      `cargo xtask ci`.
