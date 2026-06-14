## Why

The symbol decoder foundation now exists, but runtime decoder work still stops
before the AV2 § 5.20 tile payload loop and § 8.3 syntax-element CDF boundary.
The next encoder-useful step is a small tile-payload decode boundary that
validates and records the handoff from framed tile bytes to future tile syntax
without claiming reconstruction or full `decode_tile()` support.

## What Changes

- Add a source-backed `tile-payload-decode` decoder-support row tied to a new
  Feature ID for the AV2 tile payload decode boundary.
- Add crate-private `splot-decode` boundary types/functions that derive a
  borrowed tile payload plan for the constrained minimal tier: base-layer
  `OBU_CLOSED_LOOP_KEY`, complete intra first tile group, one tile, one tile
  group, bounded payload size.
- Model the per-tile § 5.20.1 handoff points: tile index, row/column, byte
  range, `tileSize`, bridge/inactive flags, current quantizer reset point,
  `init_symbol(tileSize)` eligibility, and end-of-frame CDF copyback/wrapup
  deferral.
- Add structured unsupported diagnostics for the first unsupported runtime tile
  syntax boundary, citing § 5.20 / § 8.3 and matrix row `tile-payload-decode`.
- Keep multiple tiles/tile groups, actual § 5.20.2-§ 5.20.10 block syntax,
  § 8.3 CDF-array selection,
  Tile/Saved CDF banks, coefficient decoding, prediction, reconstruction,
  hashes, runtime Y4M output, and reference refresh out of scope.
- Preserve the PR #101 concurrency model: orchestration stays in `splot-decode`
  through `DecodeContext` / `splot_parallel::WorkerPool`; `splot-core` and
  `splot-recon` stay scheduler-free.
- Do not add AVM/dav2d source, snippets, dependencies, wrappers, scripts,
  build probes, CI jobs, runtime process execution, or mandatory tests.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `decoder-support`: define and source-back the tile-payload decode boundary
  while keeping full tile syntax traversal and reconstruction unsupported.

## Impact

- Code/API: `crates/splot-decode` gains a crate-private borrowed tile-payload
  boundary plan type separate from `DecodeStreamPlan`; `splot-core` may be read
  for existing tile-group framing and symbol primitives but should not gain a
  scheduler or decode-driver dependency.
- Diagnostics/docs: `decode/unsupported-feature` remains the user-facing
  diagnostic code, with a more specific runtime tile payload reason and matrix
  row for this boundary.
- Docs/status: update `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder status,
  `docs/IMPLEMENTATION-MATRIX.toml`, generated feature/spec status, and
  `openspec/specs/decoder-support/spec.md`.
- Tests: focused `splot-decode` unit/property tests for bounded tile payload
  handoff, resource limits, unsupported diagnostics, and explicit deferral of
  `exit_symbol()` / CDF copyback until real `decode_tile()` traversal exists;
  no test may require AVM or dav2d.
- Dependencies: no new third-party dependencies and no new invalid `splot-*`
  dependency edge.
