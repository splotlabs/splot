# Change: parallelize-decode-hot-stages

## Feature IDs

- `INFRA-DECODE-PARALLEL-STAGES`

## Why

After `optimize-decode-serial-hot-paths`, `splot decode --output-format raw
--limit=1` on the motivating 1920x1080 10-bit IVF stream ran at ~309 ms with
`--threads 1` but only ~163 ms with `--threads 10` — a 1.9x speedup on 10
workers, with pool workers idle ~80% of the run. Stage timing attributes the
10-thread wall to a serial single-tile intra decode (~55 ms of interleaved
entropy parse and reconstruction), filter stages with poor internal scaling
(deblock 18.3→15.2 ms — its pass-1 column-band construction builds hundreds
of thousands of per-row slice references serially; CDEF 55.6→17.5 ms and
Wiener NS LR 119→25.7 ms — both publish their parallel outputs through
serial per-rectangle write-back loops), and a serial inter-frame decode
(~17 ms) that reconstructs each block inline during the entropy walk even
though most inter blocks read only immutable reference frames.

## What

Make the existing context-owned `WorkerPool` do real work in the measured
hot stages, preserving bit-exact output and deterministic results across
all `--threads` values:

- Extend `SPLOT_DECODE_TIMING` with per-stage runtime attribution
  (tile parse, reconstruction stages, each filter stage, inter decode) plus
  work-unit counts and distinct-worker tallies for parallel stages, so
  thread-scaling behavior is visible per stage.
- Deblock pass 1: replace the per-row column-chunk collection with
  pool-width-sized column bands so band construction is O(bands), not
  O(rows x columns); build the shared MI grid once and clone for chroma.
- CDEF: derive per-band block contexts inside row-band tasks and write
  filtered blocks directly into disjoint plane row bands, removing the
  serial context build and the serial per-chunk write-back.
- Wiener NS LR and CCSO: publish parallel per-block outputs through
  disjoint row-band writes instead of a serial write-back loop.
- Key-frame intra tile: split the interleaved walk into (1) the serial
  entropy parse that captures per-transform-unit descriptors (coefficients,
  mode facts, quantizer facts), (2) a pool-parallel dequant + inverse
  transform stage into per-unit residual buffers, and (3) a serial
  prediction/residual-add replay in exact parse order, preserving every
  neighbor-pixel, CfL, and IntraBC dependency by construction.
- Inter frames: capture placed-block descriptors during the serial entropy
  walk; compute motion-compensated prediction + residual reconstruction for
  hazard-free blocks (immutable reference reads, no current-frame reads) on
  the pool; replay all block commits in parse order so intra-in-inter,
  inter-intra, and IntraBC blocks see identical neighbor state.

## Non-goals

- No assembly, SIMD intrinsics, `unsafe`, or platform-specific code.
- No change to decoded output: every touched path stays bit-exact and
  deterministic across `--threads 1/2/4/8/10/auto`.
- No new pools, no global Rayon, no ad-hoc threads, no queues in hot loops,
  no new runtime or concurrency dependency.
- No stream-specific logic; all changes are generic decoder paths.
- No frame-level pipeline or wavefront prediction scheduler in this change;
  the serial prediction replay is the documented remaining serial fraction.

## Acceptance criteria

- `splot decode --quiet --output-format raw --limit=1` produces
  byte-identical output (sha256) before and after, at every thread count.
- `SPLOT_DECODE_TIMING=1` reports per-stage times and worker attribution
  that explain the remaining serial fraction.
- `--threads 1` wall time regresses by no more than 5%.
- `--threads 10` wall time improves materially over the 1.9x baseline, with
  the remaining serial fraction attributed to the single-tile entropy chain.
- Existing splot-parallel / splot-recon / splot-decode tests pass; new
  staged paths carry parallel-vs-serial equality tests.
- `cargo xtask ci` passes.
