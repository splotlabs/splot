## Why

The tile payload boundary now reaches `init_symbol(tileSize)` and stops before
`decode_tile()` because `splot-decode` has no ownership model for the § 8.3 tile
CDF banks that `read_symbol(cdf)` must mutate. The next encoder-useful decoder
step is a narrow, crate-private CDF-selection boundary that proves the handoff
from framed tile bytes to mutable tile CDF rows without implementing block
syntax or reconstruction.

Feature ID: `DECODE-TILE-CDF-SELECTION-BOUNDARY`.

## What Changes

- Add a crate-private `splot-decode` tile CDF boundary for the minimal
  single-tile closed-loop-key tier.
- Initialize a small owned partition-entry tile CDF subset from generated
  `splot-core` default CDF tables, enough to prove default-copy, tile-copy, row
  selection, and `SymbolDecoder::read_symbol` handoff.
- Keep CDF bank state and selector errors private to `splot-decode`; no public
  `DecodeContext` API and no new `splot-core` mutable CDF-bank API.
- Record the boundary in tile payload planning so the unsupported stop is
  specifically the § 8.3 CDF-selection / `decode_tile()` boundary.
- Update decoder roadmap, decoder support matrix/status, implementation matrix,
  and OpenSpec specs with the real partial support and residuals.
- Add self-contained tests for default copying, selector bounds, CDF update
  disablement, deterministic worker-pool execution, and copy/average policy
  calculation.

Non-goals:

- No full `decode_tile()`, recursive partition parsing, prediction,
  reconstruction, inverse transforms, loop filters, hashes, runtime Y4M output,
  reference refresh, or runtime decode success.
- No multi-tile, multi-tile-group, bridge, or BRU runtime support.
- No CDF copyback/averaging mutation after tile completion; this change only
  records the policy needed by a future tile-completion row.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or runtime
  invocation.
- No new dependency and no direct Rayon/crossbeam/global-pool/thread use.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: Add a tile CDF selection/bank boundary requirement for
  `DECODE-TILE-CDF-SELECTION-BOUNDARY` under the existing decoder support model.

## Impact

- Code: `crates/splot-decode` only for mutable CDF boundary state and tests,
  reusing `splot_core::symbol::SymbolDecoder` and generated default CDF tables.
- Docs: decoder roadmap, decoder support matrix/status, implementation matrix,
  and feature status/spec coverage generated docs as required by repo checks.
- APIs: no public API change; no dependency graph change; CLI behavior remains
  plan-only unsupported for runtime decode output.
- Diagnostics: existing `decode/unsupported-feature` remains the runtime stop,
  but the matrix/spec text distinguishes the CDF-selection boundary from the
  earlier tile payload framing boundary.
