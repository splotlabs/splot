## Why

The local decoder mission stream now reaches live Wiener NS LR transform-record
derivation but stops when a block selects active CfL chroma prediction. Advancing
the stream requires consuming AV2 §5.20.5.6 `UV_CFL_PRED` mode-info and
§5.20.7.32 `read_cfl_alphas()` syntax without claiming CfL reconstruction.

## What Changes

- Add Feature ID `DECODE-CFL-CHROMA-MODE-FRONTIER` for the active CfL
  parser/runtime frontier.
- Wire generated default CfL alpha/index/sign/MHCCP direction CDF rows into the
  tile CDF subset lifecycle.
- Extend general intra mode-info decoding so active `is_cfl` can return
  `UV_CFL_PRED` and consume `read_cfl_alphas()` in spec order for the supported
  non-lossless 4:2:0 path.
- Thread the CfL chroma mode value through the local decoder mission selectable-transform
  record path so coefficient parsing can remain symbol-synchronized.
- Update decoder support/status docs and diagnostics to make the new frontier
  explicit.

## Capabilities

### New Capabilities
- `cfl-chroma-mode-frontier`: active CfL chroma mode and alpha syntax
  consumption for the local decoder mission live transform-record frontier.

### Modified Capabilities
- `selectable-transform-records`: clarify that selectable record
  derivation depends on the active CfL chroma-mode prerequisite for the local
  stream.
- `decoder-support`: record the new partial support row, proof, and current
  fail-closed diagnostic boundary.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/cdf*`,
  `crates/splot-decode/src/tile_payload/general_intra_block.rs`, and
  `crates/splot-decode/src/runtime_minimal/wienerns_lr*`.
- Affected docs: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status/spec coverage docs, and
  OpenSpec specs for the new and modified capabilities.
- No new crate dependencies, no CLI behavior change, no AVM/dav2d invocation,
  and no successful local decoder mission decode/output claim.
