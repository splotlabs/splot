# Change: parallelize-decode-hot-stages

## Feature IDs

- `INFRA-DECODE-PARALLEL-STAGES`

## Why

After `optimize-decode-serial-hot-paths`, `splot decode --output-format raw
--limit=1` on the motivating 1920x1080 10-bit IVF stream ran at ~309 ms with
`--threads 1` but only ~163 ms with `--threads 10` — a ~1.9x speedup on 10
workers, with pool workers idle most of the run. Stage timing attributed the
10-thread wall to a serial single-tile intra decode (~48 ms of interleaved
entropy parse and neighbour-dependent reconstruction), post-reconstruction
filter stages with poor internal scaling (deblock pass 1 built hundreds of
thousands of per-row slice references serially; CDEF and Wiener NS loop
restoration published their parallel outputs through serial per-rectangle
write-back loops), and a serial single-tile inter decode.

The filter stages are the parallelisable portion: they run after
reconstruction, read immutable snapshots, and write disjoint plane regions.
The single-tile entropy decode and its raster-order neighbour-dependent
prediction are fundamentally serial for a one-tile frame and are out of scope
for this change.

## What

Make the context-owned `WorkerPool` do real work in the measured filter
stages, preserving bit-exact output and deterministic results across all
`--threads` values:

- Extend `SPLOT_DECODE_TIMING` with per-stage runtime attribution (intra
  tile, each filter stage, inter decode) plus work-unit counts and
  distinct-worker tallies for parallel stages, so thread-scaling behaviour is
  visible per stage. Add `current_pool_width`/`current_worker_index` to
  `splot-parallel` for the attribution.
- Deblock pass 1: replace the per-row column-chunk collection with
  pool-width-sized column bands so band construction is O(bands x rows), not
  O(rows x columns); build the shared MI grid once and clone for chroma.
- CDEF: split the frame planes into disjoint mutable row bands, derive each
  band's block contexts inside its task, and write filtered blocks directly
  into the band, removing the serial context build and per-chunk write-back.
- Wiener NS loop restoration and CCSO: coalesce all three planes' source runs
  in one pass, then publish filtered rectangles through row-disjoint banded
  writes (`plane_bands::publish_rect_runs_parallel`) with a serial
  `write_rect` fallback.
- Add `FrameMut::into_planes` and `PlaneMut::into_samples` to `splot-recon`
  so the disjoint band splits are expressible in safe Rust.

## Non-goals

- No assembly, SIMD intrinsics, `unsafe`, or platform-specific code.
- No change to decoded output: every touched path stays bit-exact and
  deterministic across `--threads 1/2/4/8/10/auto`.
- No new pools, no global Rayon, no ad-hoc threads, no queues in hot loops,
  no new runtime or concurrency dependency.
- No stream-specific logic; all changes are generic decoder paths.
- No parse/reconstruction decoupling, wavefront prediction scheduler, or
  deferred-reconstruction pipeline: the single-tile entropy + raster-order
  prediction spine stays serial and is the documented remaining bottleneck.

## Acceptance criteria

- `splot decode --quiet --output-format raw --limit=1` produces
  byte-identical output (sha256) before and after, at every thread count.
- `SPLOT_DECODE_TIMING=1` reports per-stage times and worker attribution
  that explain the remaining serial fraction.
- `--threads 1` wall time regresses by no more than 5%.
- The parallelisable filter portion scales at least 3x at `--threads 10`; the
  remaining serial fraction is attributed to the single-tile entropy spine.
- Existing splot-parallel / splot-recon / splot-decode tests pass; the new
  banded publication carries parallel-vs-serial equality and fallback tests.
- `cargo xtask ci` passes.
