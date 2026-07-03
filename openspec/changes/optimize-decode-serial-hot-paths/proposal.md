# Change: optimize-decode-serial-hot-paths

## Feature IDs

- `INFRA-DECODE-SERIAL-HOT-PATHS`

## Why

After `optimize-decode-filter-hot-paths`, `splot decode --output-format raw
--limit=1` on a 1920x1080 10-bit IVF stream still took ~380 ms, and a
per-thread sampling profile attributed nearly all of the wall time to the
serial decode driver: the pool-parallel filter stages were already hidden
behind it. The serial cost was dominated by avoidable per-call work, not
codec math: diagnostic `SPLOT_TRACE_*` env gates consulted through a
process-global lock per block or per symbol; a full frame-MI selectable-tx
grid allocated and `None`-filled per coding block; every MI in the tile
tested against every active loop-restoration unit through the full checked
§ 7.20.1 mapping; loop-restoration filtering dispatched per 4x4 source
block with a materialized window and several allocations each; CDEF/deblock/
CCSO reading and writing through per-sample checked accessors; ~10 heap
buffers plus a full block-state clone per nonzero transform unit; a dense
inverse transform that re-dispatched the transform type inside the innermost
multiply-accumulate and transformed all-zero rows; and motion compensation
flattening the entire reference plane into a fresh `Vec` per block per
plane before an 8-tap gather with per-tap clipping.

## What

Remove the measured serial overheads without changing any codec math, tap
tables, rounding, or clamping:

- Cache `SPLOT_TRACE_*` diagnostic env gates once per process
  (`trace_flags` module); fast-gate the per-symbol `trace::emit` hook.
- Reuse one per-thread selectable-tx grid across coding blocks with a
  record-driven reset, for both the key-frame and inter-frame walks.
- Derive each active LR unit's member blocks from two per-axis matching
  scans with the tile-constant geometry hoisted, and coalesce horizontally
  contiguous LR source blocks with identical filter-visible fields into row
  runs before filtering; filter kernels validate their padded windows once
  and run row-slice inner loops.
- CDEF: per-block interior kernel with hoisted tap constants and u16
  snapshots; deblock: stack-array line transport over direct plane slices
  with a per-pass strength memo; CCSO: per-plane offset LUT and clamped row
  slices over a borrowed source.
- Coefficient path: drop the per-TU block-state clone and DC-context-line
  clones, the preflight re-walk of internally constructed scan walks, and
  per-TU quant-buffer copies; reuse per-thread residual scratch buffers.
- Inverse transform: select the 1-D kernel once per pass, skip zero input
  coefficients and all-zero rows (bit-exact: adding zero), and use i64
  shift-algebra rounding with a proven overflow bound.
- Motion compensation: `ReferencePlaneView` borrows the reference frame's
  strided storage directly (generic over the stored sample type); the
  § 7.13.3.18 convolution gains an unscaled-step contiguous-row fast path
  with the general path kept as the fallback for scaled or clipped blocks.
- Batch § 6.16.13 raw serialization and frame-hash digest updates per row.

## Non-goals

- No assembly, SIMD intrinsics, `unsafe`, or platform-specific code.
- No change to decoded output: every touched path stays bit-exact.
- No change to filter math, tap tables, rounding, or clamping.
- No stream-specific logic; all changes are generic decoder paths.
- No new dependency, no decoder architecture rewrite, no second decode path.
- No new pools and no change to the concurrency model or `--threads`
  semantics.

## Acceptance criteria

- `splot decode --quiet --output-format raw --limit=1` produces byte-identical
  output (sha256) before and after on the motivating stream.
- Existing splot-core / splot-recon / splot-decode tests pass; rewritten
  kernels carry old-vs-new bit-exactness tests.
- Median first-frame wall time drops by at least 2x from the
  `optimize-decode-filter-hot-paths` baseline.
- `cargo xtask ci` passes.
