## Why

Before this change, the live ac0ej3 key frame reported the exact frame-level
Wiener NS frontier, but the core parser still stopped before consuming any
`read_wienerns_filter()` syntax. The ac0ej3 prefix proves a narrow luma
frame-filter-bank shape:
`NumFilterClasses == 2`, plane 0 `frame_filters_on == 1`, and no reference
filters on the intra path.

## What Changes

- Add Feature ID `DECODE-AC0EJ3-WIENERNS-BANK-FRONTIER` for the next ac0ej3
  loop-restoration parser advance.
- Parse the AV2 5.20.10.6 frame-level `read_wienerns_filter(0, 0, 0, 1)` fixed
  syntax for the intra/luma shape reached by ac0ej3, including fixed-coded match
  prediction, preserving the parsed coefficients in the core model.
- Let the ac0ej3 key frame advance from `stopped_before_wienerns_filter` to a
  complete intra frame header, then fail closed at the existing runtime
  unsupported-loop-filter boundary before tile mode-info decode or output.
- Keep loop-restoration reconstruction, LR unit syntax, inter temporal-copy
  Wiener state, entropy-coded LR unit filters, 10-bit output, and successful
  ac0ej3 decode out of scope.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `runtime`: the minimal runtime SHALL continue rejecting ac0ej3 before output,
  but the live diagnostic SHALL move past the parser-only
  `unsupported_wienerns_filter` stop to the next runtime loop-filter/tool
  boundary after a complete key-frame header parse.
- `decoder-support`: decoder support metadata SHALL track the narrow ac0ej3
  Wiener NS frame-filter-bank parser advance without claiming loop-restoration
  reconstruction or full Wiener NS decode support.

## Impact

- Code: `crates/splot-core/src/headers/frame/restoration.rs` and a new
  restoration submodule for Wiener NS frame-filter parsing; minimal runtime
  diagnostic tests as needed.
- Tests: core parser unit tests for the fixed-coded luma frame-filter bank,
  ac0ej3 CLI regression, and focused runtime checks.
- Docs/tracking: implementation matrix, decoder support matrix, generated
  status and coverage docs, and this OpenSpec change.
- No dependency, crate-boundary, encoder, public CLI, or output-format changes.
