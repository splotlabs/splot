# Tasks: parallelize-decode-hot-stages

- [ ] 1. Extend `SPLOT_DECODE_TIMING` with per-stage runtime attribution and
      distinct-worker tallies for parallel stages.
- [ ] 2. Deblock: pool-width column bands for pass 1 with O(bands)
      construction; build the MI grid once and clone for chroma overlays.
- [ ] 3. CDEF: in-band context derivation and direct disjoint row-band
      writes on the pool.
- [ ] 4. LR/CCSO: disjoint row-band parallel publication of per-block
      outputs.
- [ ] 5. Key-frame intra tile: capture per-TU descriptors during the serial
      entropy parse; pool-parallel dequant + inverse transform; serial
      prediction/add replay in parse order with equality coverage.
- [ ] 6. Inter frames: placed-block descriptor capture, pool-parallel
      hazard-free block reconstruction, parse-order commit replay.
- [ ] 7. Determinism sweep across `--threads 1/2/4/8/10/auto`; before/after
      stage table; `cargo xtask ci`.
