# Change: optimize-decode-filter-hot-paths

## Feature IDs

- `INFRA-DECODE-FILTER-HOT-PATHS`

## Why

`splot decode --output-format raw --limit=1` on a 1920x1080 10-bit IVF stream
spends ~1210 ms of a ~1234 ms total inside runtime reconstruction, and a
sampling profile attributes ~90% of that to the in-loop filter stage: the
AV2 § 7.20.3 non-separable Wiener luma/chroma filters, § 7.20.4 PC-Wiener
classification, and § 7.18 CDEF. The filter math itself is cheap; the cost is
per-tap virtual source reads. Every tap re-runs the § 7.20.2 source-sample
selector including bounds validation, subsampling shifts, checked index
arithmetic, and per-sample range checks (~68M resolver calls for one luma
plane), and CDEF re-runs per-tap mi-grid inside checks plus a per-sample
workspace write. Input read (3.2 ms), planning (1.9 ms), worker-pool build
(0.4 ms), raw serialization (6.4 ms), and durable publish (11.8 ms) are all
measured immaterial.

## What

Hoist the per-tap § 7.20.2 / § 7.18 source resolution out of the per-sample
filter loops without changing any filter math, tap tables, or rounding:

- Materialize the § 7.20.2-resolved source window once per restoration block
  (resolver called once per padded source sample instead of once per tap) and
  feed the existing filter primitives a direct-indexed lookup.
- Apply the same materialization to § 7.20.4 PC-Wiener classification reads
  and the § 7.20.3 chroma filter's chroma and luma source callbacks.
- In CDEF, gather taps from the snapshot with the § 7.18 availability check
  reduced to one precomputed plane rectangle, hoist the per-strength
  `constrain` damping adjustment out of the tap loop, and batch the per-sample
  workspace write-back into per-block `write_rect` calls. Apply the same
  snapshot/batch shape to § 7.19 CCSO.
- Run the block-independent filter stages (§ 7.18 CDEF, § 7.19 CCSO, § 7.20
  Wiener NS restoration blocks) as data-parallel maps on the context's owned
  `WorkerPool` via `splot_parallel::prelude`, publishing results serially in
  block order. A new `splot_parallel::on_multiworker_pool()` guard keeps
  direct callers (tests) on the serial path so nothing can reach Rayon's
  global pool, and a one-thread pool skips work-splitting overhead. Output is
  identical across `--threads` policies.
- Add an env-gated (`SPLOT_DECODE_TIMING`) decode phase trace on stderr so the
  attribution stays reproducible; disabled by default.

## Non-goals

- No assembly, SIMD intrinsics, `unsafe`, or platform-specific code.
- No change to decoded output: every touched path stays bit-exact.
- No change to filter math, tap tables, rounding, or clamping.
- No stream-specific logic; all changes are generic decoder paths.
- No new dependency, no decoder architecture rewrite, no second decode path.
- No new pools, no global Rayon pool, no scheduling-dependent output; the
  parallel stages use only the context's existing owned `WorkerPool`.
- No relaxation of § 7.21 output scheduling for `--limit` (held implicit
  frames still emit only when scheduling releases them).

## Acceptance criteria

- `splot decode --quiet --output-format raw --limit=1` produces byte-identical
  output (sha256) before and after on the motivating stream, and the full
  conformance-vector hash sweep is unchanged.
- Existing splot-recon / splot-decode filter tests pass unchanged.
- Median first-frame wall time drops by at least 3x or below 400 ms.
- `cargo xtask ci` passes.
