# runtime Specification

## Purpose

The `splot` codec concurrency model: a single approved data-parallel engine
(Rayon, via a local owned worker pool) and a single coarse-pipeline queue
primitive (`crossbeam-channel`, bounded only), with a user-configurable
thread-count policy and deterministic output independent of thread count. This is
non-normative concurrency infrastructure and adds no AV2 conformance coverage.

Tracked by Feature IDs: `INFRA-PARALLEL-RUNTIME-POLICY`.
## Requirements
### Requirement: single concurrency-primitives crate

Only the `splot-parallel` crate SHALL depend on `rayon` or `crossbeam-channel`.
Every other workspace crate MUST reach data parallelism and pipeline queues
through `splot-parallel`'s public API. No workspace crate SHALL depend on a
competing runtime or channel library (`tokio`, `async-std`, `futures`,
`threadpool`, `scoped_threadpool`, `flume`, `async-channel`). `splot-core` MUST
remain free of any concurrency dependency, and `splot-validate` MUST remain
single-threaded.

#### Scenario: restricted crates routed through splot-parallel

- **WHEN** a workspace crate other than `splot-parallel` needs parallelism
- **THEN** it MUST depend on `splot-parallel` and MUST NOT depend on `rayon` or
  `crossbeam-channel` directly

#### Scenario: competing runtimes rejected

- **WHEN** any workspace crate adds a dependency on a banned runtime or channel
  library
- **THEN** `cargo xtask check-concurrency-policy` SHALL fail

#### Scenario: core stays runtime-free

- **WHEN** the workspace is checked
- **THEN** `splot-core` SHALL have no concurrency dependency and `splot-validate`
  SHALL NOT depend on `splot-parallel` or any restricted parallel crate

### Requirement: bounded queues only

Pipeline queues SHALL be created only via `splot_parallel::bounded_queue` with an
explicit `QueueCapacity`. Unbounded channels (`crossbeam_channel::unbounded`, any
`unbounded_queue` helper) and `std::sync::mpsc` codec pipelines MUST NOT be used.

#### Scenario: unbounded channels rejected

- **WHEN** codec source opens an unbounded channel or a `std::sync::mpsc` pipeline
- **THEN** `cargo xtask check-concurrency-policy` SHALL fail

#### Scenario: bounded queue available

- **WHEN** a producer/consumer boundary needs a queue
- **THEN** `bounded_queue` SHALL return a bounded sender/receiver pair sized by
  `QueueCapacity`

### Requirement: thread-count policy resolved once

The runtime SHALL accept a thread count of `auto`, a fixed positive integer, or
`0` (which MUST alias to `auto`). `ThreadCount::Auto` SHALL resolve once per pool
creation via `std::thread::available_parallelism()`, falling back to `1`.
`ThreadCount::Fixed(n)` MUST require `n > 0`. The CLI flag `--threads auto|N` on
`encode` and `decode` SHALL default to `auto`.

#### Scenario: zero aliases to auto

- **WHEN** the thread count is given as `0`
- **THEN** it SHALL behave identically to `auto`

#### Scenario: auto resolves to available parallelism

- **WHEN** a pool is created with `auto`
- **THEN** the resolved worker count SHALL come from `available_parallelism()`,
  falling back to `1` when it cannot be determined

### Requirement: one local worker pool per context

Each encode and decode context SHALL own exactly one local
`splot_parallel::WorkerPool` wrapping a local Rayon thread pool. The global Rayon
pool and `ThreadPoolBuilder::build_global` MUST NOT be used. Nested parallelism
SHALL run inside `WorkerPool::install`, and code MUST NOT build nested pools, a
pool per frame/tile/superblock-row/task, or spawn ad-hoc OS threads outside tests.

#### Scenario: context owns a single pool

- **WHEN** an encode or decode context is constructed
- **THEN** it SHALL own exactly one `WorkerPool` and MUST NOT initialize the
  global Rayon pool

#### Scenario: nested work uses install

- **WHEN** parallel work needs to nest
- **THEN** it SHALL run inside `WorkerPool::install` rather than building a new
  pool

### Requirement: deterministic output independent of thread count

Observable output SHALL be identical regardless of the thread count. Future
bitstream-affecting decisions MUST NOT depend on thread scheduling; reductions
SHALL use deterministic ordering or a documented deterministic accumulation; and
output packets, decoded-frame hashes, diagnostics, progress events, and stats
SHALL be committed in presentation/bitstream order, not completion order.

#### Scenario: identical output across thread counts

- **WHEN** the same input is processed with `--threads 1`, `--threads auto`, and
  any `--threads N`
- **THEN** the observable output SHALL be byte-for-byte identical

### Requirement: enforcement by check-concurrency-policy

The concurrency policy SHALL be enforced by `cargo xtask
check-concurrency-policy`, which runs in `cargo xtask ci` and in CI, alongside
`cargo xtask check-dependency-direction` for the dependency-graph edges.

#### Scenario: policy gate runs in ci

- **WHEN** `cargo xtask ci` runs
- **THEN** `cargo xtask check-concurrency-policy` SHALL run and SHALL fail the
  build on any policy violation
