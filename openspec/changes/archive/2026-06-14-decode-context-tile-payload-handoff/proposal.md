## Why

`splot-decode` already has a crate-private tile-payload boundary that can derive
one deterministic work unit for the minimal single-tile closed-loop-key tier and
then stop at structured `decode/unsupported-feature` metadata for the
unimplemented `decode_tile()` step. That boundary is still called directly in
tests, with tests manually reaching into `DecodeContext::pool().install(...)`.

The next decoder mission slice should make the `DecodeContext` the owner of this
handoff, matching the PR #101 concurrency model: decoder orchestration runs
through the context-owned `splot_parallel::WorkerPool`, while `splot-recon`
remains scheduler-free. This gives future tile syntax and reconstruction work a
single integration point without exposing unstable tile payload types publicly.

Feature ID: `DECODE-CONTEXT-TILE-PAYLOAD-HANDOFF`.

## What Changes

- Add a crate-private `DecodeContext` method that plans the existing tile-payload
  boundary inside the context-owned `WorkerPool`.
- Keep the tile payload input, plan, work-unit, and error types crate-private;
  no public `DecodeContext` tile API is introduced in this PR.
- Replace the stale module-level dead-code allowance with conditional,
  reasoned `dead_code` allowances for non-test builds that document the
  remaining missing piece: no runtime decode path derives
  `TilePayloadBoundaryInput` facts yet.
- Update tile-payload tests so deterministic worker-pool coverage reaches the
  boundary through the new context method instead of manually calling
  `ctx.pool().install(...)`.
- Update decoder roadmap, decoder support matrix/status, implementation matrix,
  and OpenSpec specs to record the context-owned handoff and residual
  unsupported runtime behavior.
- Record PR #113 / PR #114 review carry-forward in `agent-log.md`: the prior
  byte-planner comments are already fixed on `main`, and this PR must not
  regress the `DecodeContext` documentation or concurrency boundary.

Non-goals:

- No public tile-payload API, no CLI behavior change, and no runtime `splot
  decode` success path.
- No full `decode_tile()`, recursive tile/block syntax traversal, `exit_symbol()`
  after real syntax, CDF copyback/averaging mutation, reconstruction, hashes,
  runtime Y4M output, output scheduling, or reference refresh.
- No `splot-decode -> splot-recon` dependency in this slice.
- No AVM/dav2d source, dependency, wrapper, script, CI job, required test, or
  runtime invocation.
- No new dependency and no direct Rayon/crossbeam/global-pool/thread/queue use
  outside the existing `splot_parallel::WorkerPool` contract.

## Capabilities

### New Capabilities

- None exposed publicly.

### Modified Capabilities

- `decoder-support`: Add a context-owned tile-payload handoff requirement for
  `DECODE-CONTEXT-TILE-PAYLOAD-HANDOFF` under the existing decoder support
  model.

## Impact

- Code: `crates/splot-decode/src/context.rs` and
  `crates/splot-decode/src/tile_payload.rs` tests.
- Docs: decoder roadmap, decoder support matrix/status, implementation matrix,
  and generated feature/spec status docs required by repo checks.
- APIs: crate-private only; no public API or dependency graph change.
- Diagnostics: existing tile boundary `decode/unsupported-feature` and
  `decode/resource-limit` metadata remain unchanged; this change only moves the
  handoff under `DecodeContext`.
