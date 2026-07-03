# runtime delta: optimize-decode-filter-hot-paths

Adds the callee-side parallelism guard and the decode filter-stage parallelism
constraints to the `runtime` capability. Non-normative codec-runtime
infrastructure: it adds no AV2 conformance coverage and changes no decoded
output. Tracked by `INFRA-DECODE-FILTER-HOT-PATHS`.

## ADDED Requirements

### Requirement: callee-side parallelism is pool-guarded

A codec helper SHALL gate its parallel path on
`splot_parallel::on_multiworker_pool()` and SHALL keep an equivalent serial
path whenever it is normally driven inside a context's `WorkerPool::install`
scope but is also callable directly (for example from unit tests). The guard SHALL be true only on a worker thread of an installed pool
with more than one thread. Files whose `install` scoping lives in another
file SHALL carry a documented entry in the concurrency gate's
`PAR_ITER_RULE_ALLOWLIST`. Tracked by `INFRA-DECODE-FILTER-HOT-PATHS`.

#### Scenario: direct call never reaches the global pool

- **WHEN** a pool-guarded helper is called outside any installed `WorkerPool`
- **THEN** `on_multiworker_pool()` is false and the helper runs its serial
  path without instantiating Rayon's global pool

#### Scenario: one-thread pool skips work splitting

- **WHEN** a pool-guarded helper runs inside a one-thread `WorkerPool`
- **THEN** `on_multiworker_pool()` is false and the helper runs its serial
  path without Rayon work-splitting overhead

### Requirement: decode filter stages parallelize deterministically

The decode runtime SHALL compute its block-independent in-loop filter stages
(§ 7.18 CDEF, § 7.19 CCSO, § 7.20 Wiener NS restoration blocks) from
immutable pre-stage snapshots as data-parallel maps on the context's owned
worker pool and SHALL publish results serially in block order. Decoded
output SHALL be byte-identical across `--threads 1`, `--threads auto`, and
any fixed `--threads N`. Tracked by `INFRA-DECODE-FILTER-HOT-PATHS`.

#### Scenario: output independent of thread policy

- **WHEN** the same bitstream decodes under `--threads 1` and `--threads auto`
- **THEN** raw and hash outputs are byte-identical

#### Scenario: filter block writes stay ordered

- **WHEN** a filter stage finishes computing its blocks on the pool
- **THEN** the workspace writes happen serially in the stage's block order
  over disjoint rectangles

### Requirement: decode phase timing trace is opt-in

The decode path SHALL emit its phase timing trace (input read, context
construction, planning, runtime decode, raw serialization, output publish,
total) on stderr only when `SPLOT_DECODE_TIMING` is set, and normal CLI
output SHALL be unchanged when it is unset. Tracked by
`INFRA-DECODE-FILTER-HOT-PATHS`.

#### Scenario: trace disabled by default

- **WHEN** `splot decode` runs without `SPLOT_DECODE_TIMING`
- **THEN** no `splot.decode_timing` lines are emitted

#### Scenario: trace enabled by the environment variable

- **WHEN** `splot decode` runs with `SPLOT_DECODE_TIMING=1`
- **THEN** `splot.decode_timing <phase>_ms=<value>` lines are emitted on
  stderr and stdout output is unchanged
