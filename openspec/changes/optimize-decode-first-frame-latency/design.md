# Design: optimize-decode-first-frame-latency

## Context

The current release baseline for the motivating command is already past the
previous filter and serial hot-path milestones. Warmed `SPLOT_DECODE_TIMING`
runs show input read, context construction, planning, and raw serialization are
not first-order costs for the 9.2 MiB IVF stream. Default-thread runtime decode
dominates at about 132 ms, with durable raw publish adding about 12 ms.

The short sampling profile points at the final reconstruction/filter stages:
PC-Wiener classification, CDEF interior filtering, Wiener NS luma/chroma
filtering, and result collection around those stages. `--threads 1` is much
slower than default, so the existing owned-pool parallelism is useful and should
remain intact.

## Goals / Non-Goals

**Goals:**

- Reduce first-frame latency for the existing raw/hash decode path using
  measured runtime hot-path improvements.
- Keep decoded raw bytes and hash output byte-identical across default and
  `--threads 1` policies.
- Preserve the existing opt-in timing trace and use it for before/after
  attribution.
- Record the new feature row and proof commands in the implementation matrix.

**Non-Goals:**

- No AV2 behavior, syntax, CDF, tap-table, rounding, or clamping changes.
- No assembly, intrinsics, `unsafe`, platform-specific code, or new dependency.
- No second decoder path and no stream-specific `local decoder mission` special case.
- No input/prefix planner work unless a later measurement shows it is material;
  current input and planning costs are below 2 ms warmed.
- No `--threads` semantic change or global Rayon pool.

## Decisions

1. Start with final reconstruction/filter hot paths, not I/O or planning.

   Rationale: measured input read is about 1.4 ms and planning is about 0.9 ms
   on warmed runs; optimizing them cannot close the current 132 ms runtime gap.
   The profile points directly at filter computation and collection.

2. Prefer local allocation and loop-shape reductions inside existing generic
   helpers.

   Rationale: these changes can preserve the existing public API, dependency
   graph, and correctness tests. Examples include avoiding per-row segment
   vectors in Wiener NS luma filtering, writing through already-validated output
   paths only when fail-atomic behavior is preserved, and reducing avoidable
   result-buffer growth.

3. Keep parallel publication ordering unchanged.

   Rationale: filter blocks are already computed independently and published
   serially. That ordering is part of the deterministic output contract; this
   change may reduce work inside each block but must not publish by completion
   order.

4. Treat larger classifier changes as follow-up unless first-pass timing proves
   they are required.

   Rationale: a summed-area or row-incremental PC-Wiener classifier may be the
   next significant optimization, but it is a larger algorithmic reshape of
   § 7.20.4-adjacent code. Smaller bit-exact reductions should land first if
   they give measurable improvement.

## Risks / Trade-offs

- Fail-atomic filter output could be weakened by direct writes -> only write
  directly after all relevant validation has completed; otherwise keep scratch
  buffering.
- Small local optimizations may not reach the 30 fps target -> re-measure after
  each slice and continue only down measured hotspots.
- Parallel timings can be noisy on a short command -> use one warm-up and at
  least five warmed runs, reporting median and best.
- Optimizing filter internals can accidentally change edge behavior -> keep
  existing bit-exact tests and add targeted equivalence tests when a loop shape
  changes.
