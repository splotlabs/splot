# Tasks: parallelize-decode-hot-stages

- [x] 1. Extend `SPLOT_DECODE_TIMING` with per-stage runtime attribution and
      distinct-worker tallies; add `current_pool_width` /
      `current_worker_index` to `splot-parallel`.
- [x] 2. Deblock: pool-width column bands for pass 1 with O(bands x rows)
      construction; build the MI grid once and clone for chroma overlays.
- [x] 3. CDEF: disjoint plane row-band split (`FrameMut::into_planes` +
      `PlaneMut::into_samples`), in-band context derivation, direct band
      writes on the pool.
- [x] 4. LR/CCSO: one-pass all-plane run coalescing; row-disjoint banded
      publication (`plane_bands::publish_rect_runs_parallel`) with a serial
      `write_rect` fallback and parallel-vs-serial + fallback tests.
- [x] 5. Determinism sweep across `--threads 1/2/4/8/10/auto`; before/after
      stage table with Amdahl breakdown; `cargo xtask ci`.
- [ ] 6. (Deferred, out of scope) Parse/reconstruction decoupling to
      parallelise the single-tile entropy + prediction spine; documented as
      the next bottleneck, requires architecture-scale change.
