## ADDED Requirements

### Requirement: Decoder runtime concurrency contract

The decoder support model SHALL incorporate the repository runtime concurrency
policy tracked by `INFRA-PARALLEL-RUNTIME-POLICY` before any byte-consuming
decode, reconstruction, deterministic frame-hash, Y4M-output, reference-update,
or encoder roundtrip row is marked supported. Future decoder work SHALL use the
single approved `splot_parallel` model: one `WorkerPool` owned by each
`splot-decode` context, parallel iterators reached through
`splot_parallel::prelude` and driven inside `WorkerPool::install`, and bounded
queues only through `splot_parallel::bounded_queue` at coarse pipeline
boundaries. `splot-recon` SHALL remain pool-agnostic reconstruction and data
model infrastructure; it MUST NOT construct worker pools, use direct Rayon or
crossbeam APIs, spawn codec worker threads, or own pipeline queues. Observable
decoder output SHALL remain deterministic across `--threads 1`,
`--threads auto`, and fixed positive `--threads N`.

#### Scenario: Decoder roadmap documents the runtime policy

- **WHEN** a reader opens `docs/DECODER-ROADMAP.md`
- **THEN** the roadmap links future decoder/reconstruction work to
  `INFRA-PARALLEL-RUNTIME-POLICY` and `docs/CONCURRENCY.md`
- **AND** it states that `splot-decode` owns runtime orchestration through a
  single context-owned `WorkerPool`
- **AND** it states that `splot-recon` remains pool-agnostic and reusable by the
  future encoder

#### Scenario: Future parallel decode work uses the context pool

- **WHEN** future decode or reconstruction orchestration adds data-parallel work
- **THEN** it MUST reach parallel iterator traits through
  `splot_parallel::prelude`
- **AND** it MUST run those iterators inside the owning decode context's
  `WorkerPool::install`
- **AND** it MUST NOT build a nested pool, initialize the Rayon global pool,
  spawn ad-hoc codec worker threads, or depend on `rayon` outside
  `splot-parallel`

#### Scenario: Reconstruction primitives stay pool-agnostic

- **WHEN** `splot-recon` gains reconstruction, reference, hash, or output helper
  APIs
- **THEN** those APIs MUST remain callable without constructing or owning a
  worker pool
- **AND** any parallel scheduling wrapper MUST live in `splot-decode` or another
  caller that already owns the context runtime policy
- **AND** `splot-recon` MUST NOT depend directly on `rayon`,
  `crossbeam-channel`, or another runtime/channel crate

#### Scenario: Queues are bounded coarse pipeline boundaries

- **WHEN** future byte-consuming decode needs a producer/consumer boundary
- **THEN** it MUST use `splot_parallel::bounded_queue` with an explicit
  `QueueCapacity`
- **AND** it MUST NOT use unbounded channels, `std::sync::mpsc`, or queues for
  per-pixel, per-block, per-row, or other hot inner-loop signalling

#### Scenario: Decode output is deterministic across thread counts

- **WHEN** a future decode row claims runtime support for decoded-frame hashes,
  Y4M output, diagnostics, stats, reference updates, or another observable
  artifact
- **THEN** its proof MUST include self-contained evidence that observable output
  is committed in AV2 bitstream, presentation, or repository-owned emission
  order rather than worker completion order
- **AND** its tests MUST cover the supported behavior across all required
  thread-count forms: `--threads 1`, `--threads auto`, and at least one fixed
  positive `--threads N`

#### Scenario: Current unsupported decode remains honest

- **WHEN** this contract is added before runtime decode support exists
- **THEN** `splot decode` continues to emit the existing
  `decode/unsupported-feature` diagnostic for valid invocations
- **AND** no byte-consuming decode path, reconstruction algorithm, frame-hash
  digest computation, Y4M output, AVM/dav2d invocation, or new emitted decoder
  diagnostic is claimed
