# Tasks: optimize-decode-filter-hot-paths

- [x] 1. Env-gated `SPLOT_DECODE_TIMING` phase trace (input read, context,
      plan, runtime decode, raw serialize, publish, total) on stderr.
- [x] 2. Luma § 7.20.3 Wiener NS: materialize the § 7.20.2-resolved padded
      source window once per restoration block; padded-source filter entry
      point with precomputed tap offsets.
- [x] 3. § 7.20.4 PC-Wiener classification: reuse the materialized window for
      classification reads.
- [x] 4. Chroma § 7.20.3 Wiener NS: materialized chroma and luma companion
      windows.
- [x] 5. § 7.18 CDEF: bulk snapshot capture, one precomputed inside rectangle,
      per-block tap offsets, hoisted constrain damping, batched write-back.
      Same snapshot/batch shape for § 7.19 CCSO.
- [x] 6. Run block-independent filter stages on the context's owned pool
      behind `splot_parallel::on_multiworker_pool()`, publishing serially in
      block order; identical output across `--threads` policies.
- [x] 7. Re-measure phase table; confirm bit-exact raw sha256 and
      conformance-vector sweep; run `cargo xtask ci`.
