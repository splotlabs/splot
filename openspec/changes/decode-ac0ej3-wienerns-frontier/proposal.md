## Why

The live ac0ej3 decode now advances past the sequence chroma-tool gate, but the
minimal runtime still reports the parser's `StoppedBeforeWienerNsFilter` status
as a generic `incomplete_frame_header`. That hides the true next frontier: AV2
5.18.7.11 entering the unmodeled frame-level `read_wienerns_filter()` bank decode.

## What Changes

- Add Feature ID `DECODE-AC0EJ3-WIENERNS-FRONTIER` for the ac0ej3 Wiener NS
  frame-header frontier.
- Map `FrameHeaderParseStatus::StoppedBeforeWienerNsFilter` to a precise
  `decode/unsupported-feature` diagnostic before any tile mode-info decode,
  sample allocation, or output.
- Update the local ac0ej3 CLI regression and decoder tracking docs to expect
  `unsupported_wienerns_filter` at byte offset 74.
- Keep the parser and reconstruction capability unchanged; this is a fail-closed
  diagnostic/tracking brick only.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `runtime`: the minimal runtime SHALL surface the ac0ej3 Wiener NS parser stop as
  a precise unsupported-feature diagnostic instead of the generic incomplete
  header fallback.
- `decoder-support`: decoder support metadata SHALL track the ac0ej3 Wiener NS
  frontier without claiming Wiener NS filter-bank parsing or loop-restoration
  reconstruction support.

## Impact

- Code: `crates/splot-decode/src/runtime_minimal.rs`.
- Tests: focused runtime diagnostic tests plus the ignored local ac0ej3 CLI
  regression.
- Docs/tracking: implementation matrix, decoder support matrix, generated status
  and coverage docs, and this OpenSpec change.
- No dependency, crate-boundary, encoder, bitstream parser, or public API changes.
