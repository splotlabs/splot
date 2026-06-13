# Design: parallel-runtime-policy

Tracks `INFRA-PARALLEL-RUNTIME-POLICY`.

## Context

`splot` will eventually run real reconstruction and encoding, which need data
parallelism and coarse pipeline concurrency. Rather than let concurrency
dependencies and patterns proliferate, the workspace concentrates them in one
crate, `splot-parallel`, with a single thread-count policy and a single bounded
queue primitive. This keeps the spec model (`splot-core`) and the validator
(`splot-validate`) runtime-free and keeps the supply-chain surface to exactly two
authorized crates.

This is **original infrastructure**. It is not derived from rav1e, SVT-AV1, AVM,
or dav1d/dav2d source, syntax, tables, constants, entropy CDFs, comments, or
prose. It models no AV2 syntax or semantics and therefore does not implicate the
encoder reference-gate copying rules; it is generic Rust concurrency plumbing
built on the public `rayon`, `crossbeam-channel`, and `std` APIs.

## Data model / API

- `ThreadCount` — `Auto` (default) or `Fixed(NonZeroUsize)`. `from_count_or_auto`
  maps `0 -> Auto` and `n > 0 -> Fixed(n)`. `resolve()` returns a
  `NonZeroUsize`: `Auto` uses `std::thread::available_parallelism()` (fallback
  `NonZeroUsize::MIN`), resolved once per pool creation. `FromStr` accepts
  `auto`, `0` (alias), and positive integers.
- `WorkerPool` — wraps a private, owned `rayon::ThreadPool` built with a local
  `ThreadPoolBuilder` (never `build_global`), with worker threads named
  `splot-worker-{i}`. Exposes `new(ThreadCount)`, `threads() -> NonZeroUsize`,
  `requested() -> ThreadCount`, and `install(f)` to scope nested Rayon work to the
  owned pool.
- `bounded_queue(QueueCapacity) -> (QueueSender, QueueReceiver)` — the only queue
  constructor; wraps `crossbeam-channel`'s bounded channel. No unbounded variant
  is exposed.
- `EncoderRuntimeConfig` / `DecodeRuntimeConfig` carry a `ThreadCount` (default
  `Auto`); `Context`/`DecodeContext` each own exactly one `WorkerPool`.

## Dependency graph

`splot-parallel` depends on no other `splot-*` crate. Real edges today:
`splot-encode -> splot-core + splot-parallel`, `splot-decode -> splot-parallel`
(approved future: `splot-core`, `splot-recon`), `splot-cli -> splot-parallel`
(plus existing). Only `splot-parallel` depends on `rayon` or
`crossbeam-channel`. Enforced by `cargo xtask check-dependency-direction` and
`cargo xtask check-concurrency-policy`.

## Determinism contract

Future bitstream-affecting decisions must not depend on thread scheduling. Output
must be identical across `--threads 1`, `auto`, and any `N`. Reductions use
deterministic ordering or a documented deterministic accumulation; per-frame /
per-tile work writes into disjoint regions or local buffers merged in stable
order; output packets, decoded-frame hashes, diagnostics, progress events, and
stats are committed in presentation/bitstream order, not completion order. Full
policy: `docs/CONCURRENCY.md`.

## Why Rayon + crossbeam-channel

- **Rayon** is the de facto Rust data-parallel work-stealing engine; using a
  *local owned* pool (not the global one) gives bounded, context-scoped
  parallelism without a process-wide singleton.
- **crossbeam-channel** provides well-tested bounded MPMC channels with
  backpressure, which suit coarse producer/consumer pipeline boundaries far better
  than `std::sync::mpsc`. Unbounded channels are deliberately not exposed so
  memory cannot grow without bound.
- Async runtimes (tokio, async-std, futures) are rejected: a CPU-bound codec needs
  data parallelism and backpressure, not an async executor, and they would add a
  large, unnecessary dependency surface.

## Enforcement

`cargo xtask check-concurrency-policy` (in `cargo xtask ci` and CI) restricts
`rayon`/`crossbeam-channel` to `splot-parallel`, bans the competing
runtime/channel crates, keeps `splot-core` runtime-free and `splot-validate`
single-threaded, and scans `crates/**/*.rs` (test-aware) for `build_global`,
unbounded channels, `std::sync::mpsc` pipelines, and non-test `thread::spawn`.

## Alternatives considered

- **Async runtime (tokio/async-std):** rejected — wrong tool for CPU-bound codec
  work; large dependency surface.
- **`std::sync::mpsc` only:** rejected — no bounded MPMC with backpressure that
  fits the planned pipeline.
- **Global Rayon pool:** rejected — a process-wide singleton breaks per-context
  ownership and deterministic, scoped parallelism.

## Risks

- Spec ambiguity: none — non-normative infrastructure.
- Performance: none yet; no codec stage runs in parallel today.
- Compatibility: none — no public codec behavior changed.
- Maintenance: the gate plus this policy keep the concurrency surface small and
  reviewable.
