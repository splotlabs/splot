## Context

`splot-decode` currently frames one minimal-tier tile payload, checks resource
limits, derives a deterministic work unit, initializes AV2 § 8.2 symbol state,
and stops before `decode_tile()` / § 8.3 CDF selection. `splot-core` already owns
the generic `SymbolDecoder` and generated default CDF tables; it does not own a
runtime decode pipeline or mutable tile CDF banks. The runtime concurrency policy
from PR #101 requires decode orchestration to stay in `splot-decode` through
`DecodeContext` / `splot_parallel::WorkerPool`, while `splot-recon` remains
scheduler-free.

This change adds the next boundary without claiming full tile syntax support:
`splot-decode` will own a crate-private CDF-bank subset copied from generated
defaults and expose a narrow row-selection handoff to
`SymbolDecoder::read_symbol(&mut [i32])`.

Spec anchors:

- § 5.20.1 tile payload framing calls `init_symbol(tileSize)`, `decode_tile()`,
  and later `exit_symbol()`:
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1`.
- § 5.20.2.1 and § 5.20.3.2 define `decode_tile()` / `read_partition()` entry
  territory that remains unsupported:
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-2-1` and
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2`.
- § 8.2 owns symbol initialization/read/exit and frame-end CDF copy/average
  policy:
  `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2`.
- § 8.3 says `S` syntax elements choose a mutable CDF row by reference before
  calling `read_symbol(cdf)`:
  `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3`.
- § 9.3 generated default CDF tables are the source for initial row values:
  `docs/spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md`.

## Goals / Non-Goals

**Goals:**

- Add crate-private `splot-decode` CDF boundary types with stable
  `DECODE-TILE-CDF-SELECTION-BOUNDARY` tracking.
- Copy a deliberately small partition-entry CDF subset from generated defaults:
  `Default_Do_Split_Cdf` and `Default_Do_Square_Split_Cdf`.
- Provide typed selectors and errors for that subset, using closure-based
  mutable row access so future tile syntax code can call `SymbolDecoder` without
  leaking long-lived mutable aliases.
- Compute and test the frame-end copy/average decision from
  `enable_avg_cdf`, `avg_cdf_type`, `context_update_tile_id`, `TileNum`, and
  `TileCols * TileRows`, without applying saved-bank mutation yet.
- Attach the boundary to the existing tile payload work unit as metadata and keep
  the runtime unsupported diagnostic at the `decode_tile()` boundary.
- Prove deterministic behavior through `DecodeContext` worker-pool policies
  without adding direct Rayon/crossbeam usage.

**Non-Goals:**

- No public `DecodeContext` API change.
- No new `splot-core` mutable CDF-bank module; `splot-core` remains the source
  for generated defaults and § 8.2 symbol primitives.
- No full § 8.3 selector table, full CDF bank, recursive `decode_partition()`,
  block syntax, reconstruction, hashes, Y4M output, reference refresh, or runtime
  decode success.
- No AVM/dav2d repo integration or required local reference run.
- No dependency graph change and no direct scheduler/thread/queue ownership.

## Decisions

1. **Keep the mutable CDF boundary in `splot-decode`.**

   The CDF banks are decode-runtime state, not a pure parser primitive. Keeping
   them crate-private in `splot-decode` preserves `splot-core` as syntax/model
   and generated-table owner while avoiding a premature public CDF-bank API.

2. **Start with a narrow generated-default subset.**

   `DoSplitCdf` and `DoSquareSplitCdf` are the first partition-entry CDF rows
   reached by § 5.20 `decode_tile()` / `read_partition()` territory. They are
   enough to prove default copy, selector bounds, CDF update enable/disable, and
   `read_symbol` handoff without duplicating the entire § 9.3 table surface or
   claiming recursive partition traversal.

3. **Use typed selector inputs plus closure-based row access.**

   A `TileCdfSelector` enum should accept typed contexts rather than raw nested
   indexes wherever possible. `with_row_mut(selector, |row| ...)` avoids exposing
   long-lived mutable row references and keeps future syntax traversal borrow
   scopes local.

4. **Record copy/average policy; do not mutate saved banks after tile exit.**

   § 8.2 frame-end CDF update depends on actual tile completion and
   `exit_symbol()`, both still unsupported. This change may compute
   `copyCdf`/`avgCdf` for a work unit and keep a small saved subset for tests,
   but it must not claim frame-end CDF update support.

5. **Keep concurrency at the `DecodeContext` boundary.**

   Tests that need runtime-policy proof should call the crate-private boundary
   inside `ctx.pool().install(...)`, matching existing tile payload tests. The
   new module must not import Rayon/crossbeam directly or construct worker pools.

## Risks / Trade-offs

- **Risk: subset looks like full § 8.3 support.** Mitigation: name the feature
  and docs as a boundary, keep module crate-private, use a tiny selector subset,
  and leave runtime unsupported metadata at `decode_tile()`.
- **Risk: raw indexes can panic.** Mitigation: typed bounded wrappers and
  selector validation return typed errors before indexing.
- **Risk: CDF values diverge from the spec.** Mitigation: copy only from
  generated `splot-core::tables::cdf` statics; never hand-transcribe table
  contents.
- **Risk: copy/average policy is mistaken for frame-end support.** Mitigation:
  tests assert policy calculation only; docs and matrix state that saved-bank
  mutation after real tile completion is residual.
- **Risk: local reference tooling leaks into repo.** Mitigation: no AVM/dav2d
  commands, scripts, dependencies, or CI changes are needed for this boundary.
