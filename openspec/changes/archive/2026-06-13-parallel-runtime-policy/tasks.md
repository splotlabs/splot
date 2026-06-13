# Tasks

## Implementation

- [x] Add the `splot-parallel` crate with `ThreadCount` (`auto` default, fixed
      positive integer, `0` aliases to `auto`; resolved once via
      `available_parallelism`, fallback 1).
- [x] Add `WorkerPool` wrapping a local Rayon `ThreadPool` (never the global pool
      or `build_global`; worker threads named `splot-worker-{i}`) with `new`,
      `threads`, `requested`, and `install`.
- [x] Add bounded `crossbeam-channel` queues only (`bounded_queue`,
      `QueueCapacity`, `QueueSender`/`QueueReceiver`); do not expose unbounded.
- [x] Wire `EncoderRuntimeConfig` (default `Auto`) and `Context::new` to own one
      `WorkerPool`; keep frame ops returning the unchanged `Unimplemented` error.
- [x] Wire `DecodeRuntimeConfig` and a `DecodeContext` pool scaffold that reads no
      bytes; keep `decode/unsupported-feature` unchanged.
- [x] Add `splot encode|decode --threads auto|N` (default `auto`).
- [x] Add the `splot-parallel` graph edges (`splot-encode -> splot-parallel`,
      `splot-decode -> splot-parallel`, `splot-cli -> splot-parallel`).

## Tests and proof

- [x] Positive `ThreadCount` parse/resolve/alias tests.
- [x] `WorkerPool` thread-count and worker-name tests.
- [x] Bounded-queue send/receive tests.
- [x] `check-concurrency-policy` evaluator unit tests against synthetic fixtures.

## Documentation and tracking

- [x] Add the `INFRA-PARALLEL-RUNTIME-POLICY` matrix row and regenerate
      `docs/FEATURE-STATUS.md` and `docs/SPEC-COVERAGE.md`.
- [x] Add `docs/CONCURRENCY.md` and the concurrency pointers/checklist in
      `docs/ARCHITECTURE.md` and `docs/CODE_REVIEW.md`.
- [x] Update `README.md` and `AGENTS.md` (repository map, one-way dependency
      rules, command list).

## Checks

- [x] `cargo test -p splot-parallel --all-targets --locked`
- [x] `cargo xtask check-concurrency-policy`
- [x] `cargo xtask check-dependency-direction`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
