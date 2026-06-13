# Change: parallel-runtime-policy

## Feature IDs

- `INFRA-PARALLEL-RUNTIME-POLICY`

## Why

`splot` is validator-first today, but reconstruction and encoding will need
data-parallel and pipeline concurrency. Letting concurrency dependencies and
patterns spread ad hoc across the workspace would make review, determinism, and
supply-chain control much harder. The maintainer authorized adding exactly two
runtime crates — `rayon` and `crossbeam-channel` — funneled through a single new
crate, `splot-parallel`, so the workspace has one reviewed, deterministic,
runtime-free-by-default concurrency surface before any real codec work lands.

## Scope

- Spec sections: none newly modeled (this is non-normative infrastructure).
- Crates/modules: `splot-parallel` (new; `ThreadCount`, `WorkerPool`, bounded
  `bounded_queue`), `splot-encode` (`EncoderRuntimeConfig`, `Context` owns one
  pool), `splot-decode` (`DecodeRuntimeConfig`, `DecodeContext` pool scaffold),
  `splot-cli` (`encode`/`decode` `--threads`), `xtask`
  (`check-concurrency-policy`).
- CLI/docs/tests: `splot encode|decode --threads auto|N`; `docs/CONCURRENCY.md`,
  `docs/ARCHITECTURE.md`, `docs/CODE_REVIEW.md`, `README.md`, `AGENTS.md`;
  positive unit tests in `splot-parallel` and the gate's evaluator tests.

## Non-goals

- No real decoder or encoder; encode/decode operations stay stubs.
- No AV2 conformance behavior change (no parser, diagnostic, or bitstream output
  change); `decode/unsupported-feature` is unchanged.
- No other runtime or channel dependency (tokio/async-std/futures/threadpool/
  flume/async-channel/…).
- No global Rayon pool, no `build_global`, no unbounded channels, no
  `std::sync::mpsc` codec pipelines, no ad-hoc `thread::spawn` outside tests.

## Acceptance criteria

- [x] `INFRA-PARALLEL-RUNTIME-POLICY` row exists in
      `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] The `splot-parallel` public API (`ThreadCount`, `WorkerPool`,
      `bounded_queue`) is documented.
- [x] `splot encode` and `splot decode` accept `--threads auto|N` (default
      `auto`).
- [x] `cargo xtask check-concurrency-policy` is wired into `cargo xtask ci` and
      `.github/workflows/ci.yml`.
- [x] Positive unit tests cover `ThreadCount` resolution and `WorkerPool`
      behavior.
- [x] `cargo xtask check-feature-status` passes.
- [x] `cargo xtask check-concurrency-policy` passes.
