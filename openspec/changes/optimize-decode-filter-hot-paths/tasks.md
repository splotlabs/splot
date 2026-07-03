# Tasks: optimize-decode-filter-hot-paths

- [ ] 1. Env-gated `SPLOT_DECODE_TIMING` phase trace (input read, context,
      plan, runtime decode, raw serialize, publish, total) on stderr.
- [ ] 2. Luma § 7.20.3 Wiener NS: materialize the § 7.20.2-resolved padded
      source window once per restoration block; direct-indexed source closure.
- [ ] 3. § 7.20.4 PC-Wiener classification: reuse the materialized window for
      classification reads.
- [ ] 4. Chroma § 7.20.3 Wiener NS: materialized chroma and luma companion
      windows.
- [ ] 5. § 7.18 CDEF: per-block tap tile with precomputed availability;
      batched write-back.
- [ ] 6. Re-measure phase table; confirm bit-exact raw sha256 and
      conformance-vector sweep; run `cargo xtask ci`.
