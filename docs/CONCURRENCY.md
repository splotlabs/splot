# Concurrency

Feature ID: `INFRA-PARALLEL-RUNTIME-POLICY`.

This document is the canonical concurrency policy for `splot`. It is enforced by
`cargo xtask check-concurrency-policy` and cross-referenced by the
[CODE_REVIEW.md](./CODE_REVIEW.md) `## Concurrency` checklist and the
[ARCHITECTURE.md](./ARCHITECTURE.md) concurrency-runtime subsection.

## 1. Why this exists (and what it is not)

`splot` is **validator-first**. Today no real decoder or encoder runs: encode and
decode operations are stubs that return `Error::Unimplemented { feature: "AV2
encoder" }` or the unchanged `decode/unsupported-feature` diagnostic. The
concurrency primitives described here are **forward-looking runtime
infrastructure** so that, when reconstruction and encoding land, the workspace
already has a single, reviewed, deterministic concurrency surface. **No AV2
conformance behavior depends on any of this yet.** Adding the primitives changed
no parser, no diagnostic, and no bitstream output.

The maintainer authorized adding exactly two dependencies — `rayon` and
`crossbeam-channel` — and the dependency-graph edges that route them through one
new crate, `splot-parallel`.

## 2. Approved primitives

There are exactly two approved concurrency primitives, and **only the
`splot-parallel` crate may depend on `rayon` or `crossbeam-channel`.** Every other
crate reaches parallelism through `splot-parallel`'s API.

### 2.1 Rayon — data parallelism via a local `WorkerPool`

- Rayon is used **only** through a *local, owned* `splot_parallel::WorkerPool`,
  which wraps a private `rayon::ThreadPool`. Each encode/decode context owns
  exactly one `WorkerPool`.
- The global Rayon pool and `rayon::ThreadPoolBuilder::build_global` are **never**
  used. Nothing installs a process-global pool.
- Worker threads are named `splot-worker-{i}`.
- Parallel work runs inside `WorkerPool::install`, which scopes Rayon's
  thread-local pool to the owned pool for the duration of the closure.

### 2.2 crossbeam-channel — coarse pipeline boundaries, bounded only

- `crossbeam-channel` is used **only** via `splot_parallel::bounded_queue`, which
  returns a bounded `(QueueSender, QueueReceiver)` pair sized by `QueueCapacity`.
- Channels are for **coarse producer/consumer pipeline boundaries** (for example a
  future frame/tile pipeline stage handoff), never for fine-grained per-pixel,
  per-block, or per-row signalling.
- Unbounded channels are not exposed and not permitted.

## 3. Banned

The following are banned anywhere under `crates/` (the gate scans crate source,
test-aware):

- **Competing runtimes / libraries:** `tokio`, `futures` (`futures-core`,
  `futures-util`, `futures-executor`), `async-std`, `threadpool`,
  `scoped_threadpool`, `flume`, `async-channel`. No crate may depend on any of
  them.
- **`std::sync::mpsc` for codec pipelines** — use a bounded crossbeam queue
  instead.
- **Unbounded channels** — `crossbeam_channel::unbounded` and any
  `unbounded_queue` helper.
- **Ad-hoc `thread::spawn`** outside tests — use the local `WorkerPool`.
- **The Rayon global pool / `ThreadPoolBuilder::build_global`** — use a local
  owned `WorkerPool`.
- **Nested pools** — never build a pool inside a pool; nest work with
  `WorkerPool::install`.
- **A pool per frame / tile / superblock-row / task** — create the pool once per
  context and reuse it.
- **Channels in hot per-pixel / per-block / per-row loops** — keep channels at
  coarse pipeline boundaries.

## 4. Thread-count model

The CLI exposes `--threads auto|N` on both `splot encode` and `splot decode`
(default `auto`). The policy is modeled by `splot_parallel::ThreadCount`:

- `auto` (default) — `ThreadCount::Auto`.
- A fixed positive integer `N` — `ThreadCount::Fixed(NonZeroUsize)`; `N` must be
  `> 0`.
- `0` — aliases to `auto` (`ThreadCount::from_count_or_auto(0) == Auto`).

`ThreadCount::Auto` resolves **once per pool creation** via
`std::thread::available_parallelism()`, falling back to `1` when the platform
cannot report it. `ThreadCount::Fixed(n)` uses `n` directly. The requested
`ThreadCount` is recorded on the pool (`WorkerPool::requested`) and the resolved
worker count is available as `WorkerPool::threads`.

## 5. Ownership and nesting

- Each encode/decode context owns **exactly one** `WorkerPool`. `splot-encode`'s
  `Context` and `splot-decode`'s `DecodeContext` each construct one pool from
  their runtime config (`EncoderRuntimeConfig` / `DecodeRuntimeConfig`).
- Nested parallelism runs **inside** `WorkerPool::install`. Never build a nested
  pool to get more parallelism; the owned pool already covers the context.

### 5.1 Writing parallel code (the required pattern)

There is exactly one sanctioned way to add data-parallel work, and new code MUST
follow it so it scales with `--threads` and stays deterministic:

1. **Reach Rayon only through `splot_parallel::prelude`.** Downstream codec crates
   (`splot-encode`, `splot-decode`, …) MUST NOT depend on `rayon` directly — the
   concurrency gate forbids it. The prelude re-exports the parallel-iterator traits
   (`IntoParallelIterator`, `IndexedParallelIterator`, `ParallelIterator`,
   `ParallelSlice`, …) so you get `par_iter()` without a direct `rayon` dependency.
2. **Drive every parallel iterator inside `WorkerPool::install`.** A parallel
   iterator written at the top level runs on Rayon's *global* pool — it will not
   use the context's configured workers and will not scale with `--threads`. The
   gate flags a file that uses a `par_iter` / `par_chunks` / `par_bridge` call but
   never calls `install`.
3. **Use indexed iterators and collect in order** for determinism (see §6).

The canonical shape (works from any codec crate; no direct `rayon` dependency):

```rust,ignore
use splot_parallel::prelude::*;

let outputs: Vec<Output> = ctx.pool().install(|| {
    items
        .par_iter()
        .map(process_one) // pure; writes only to its own Output
        .collect()        // indexed source → results land in input order
});
```

Threads follow the same rule: never `std::thread::spawn` for codec work — every
worker thread comes from the context's single `WorkerPool`, sized by `--threads`.

## 6. Determinism contract

Concurrency must never change observable output. Once real decode/encode work
exists:

- **No bitstream-affecting decision may depend on thread scheduling.** Output must
  be identical across `--threads 1`, `--threads auto`, and any `--threads N`.
- **Reductions use deterministic ordering** or a documented deterministic
  accumulation — never an order that depends on which worker finished first.
- **Per-frame / per-tile work writes into disjoint regions or local buffers** that
  are merged back in a stable (index/presentation) order.
- **Output packets, decoded-frame hashes, diagnostics, progress events, and stats
  are committed in presentation/bitstream order, not completion order.**

A future-shape illustration (illustrative only — not a compiled doctest):

```rust,ignore
use splot_parallel::WorkerPool;
use splot_parallel::prelude::*; // curated par-iter traits — never a direct `rayon` dep

// Process N items in parallel inside the owned pool, then collect results in a
// deterministic, index-ordered Vec — completion order is discarded.
fn run(pool: &WorkerPool, items: &[Item]) -> Vec<Output> {
    pool.install(|| {
        items
            .par_iter()
            .map(process_one) // pure, writes only to its own Output
            .collect() // par_iter().collect() preserves input order
    })
}
```

## 7. Enforcement

`cargo xtask check-concurrency-policy` (run in `cargo xtask ci` and CI) enforces:

- Only `splot-parallel` may depend on `rayon` or `crossbeam-channel`.
- No crate depends on a banned runtime/channel library (`tokio`, `async-std`,
  the whole `futures`/`futures-*` family, `threadpool`, `scoped_threadpool`,
  `flume`, `async-channel`). Dependency names are resolved through
  `package = "..."` and `workspace = true` aliases, so a banned crate cannot hide
  behind a rename.
- `splot-core` stays runtime-free (no concurrency dependency at all).
- `splot-validate` stays single-threaded (no `splot-parallel` or restricted
  parallel dependency).
- Codec source under `crates/` does not use the global Rayon pool — neither
  `build_global` nor the `rayon::spawn` / `rayon::join` / `rayon::scope` (and
  related) free functions — open an unbounded channel (any import/call/alias form
  of `crossbeam_channel::unbounded` / `unbounded_queue`), build a
  `std::sync::mpsc` pipeline, or spawn ad-hoc OS threads (`thread::spawn`,
  `thread::Builder`, or a `std::thread` alias) outside tests.
- Aliased imports that could hide one of those calls (e.g. `use std::thread as t;`
  or `use crossbeam_channel as cc;`) are flagged at the rename declaration.
- Outside `splot-parallel`, a Rayon parallel-iteration call (`par_iter`,
  `par_chunks`, `par_bridge`) must sit inside `WorkerPool::install`: a file that
  uses one but never calls `install` is flagged, since it would run on the global
  pool and not scale with `--threads`.

The source scan is a line-based **defense-in-depth** check: it does not perform
full syntactic alias resolution, so a deliberately obfuscated multi-hop re-export
could still evade it. The dependency-graph edges are independently enforced by
`cargo xtask check-dependency-direction`, and the
[CODE_REVIEW.md](./CODE_REVIEW.md) `## Concurrency` checklist is the per-change
human backstop for anything the line scanner cannot see.
