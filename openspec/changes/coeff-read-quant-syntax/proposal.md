## Why

The ordinary non-FSC coefficient path can now read EOB, walk scan order, read
base/sign summaries, and apply caller-provided quantized values, but it still
lacks the AV2 § 5.20.7.28 `read_quant` syntax boundary that produces those
values. Adding that boundary is the next narrow step toward a real
§ 5.20.7.27 `coeffs()` loop without widening runtime decode support.

## What Changes

- Add Feature ID `DECODE-COEFF-READ-QUANT-SYNTAX`.
- Add a crate-private `splot-decode` helper for AV2 § 5.20.7.28
  `read_quant`, over caller-resolved level, position, hidden-parity,
  max-level, `hrLevelAvg`, and TCQ facts.
- Consume only literal bits from the tile symbol decoder as specified by the
  q-length, Golomb-length, and coefficient-remainder syntax.
- Return the decoded `quant` value and updated `hrLevelAvg` for the existing
  quant-state writer, while keeping runtime `coeffs()` integration and decode
  output unchanged.
- Keep broad coefficient-loop wiring, dequantization, inverse transforms,
  residual add, reconstruction, and reference refresh out of scope.

## Capabilities

### New Capabilities

- `coeff-read-quant-syntax`: crate-private ordinary non-FSC AV2 § 5.20.7.28
  `read_quant` syntax parsing for coefficient-loop callers.

### Modified Capabilities

- `decoder-support`: records the new decoder-support row and keeps broad
  coefficient decode/reconstruction rows honest while runtime wiring remains
  partial.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/` and focused
  coefficient-loop tests.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated feature/support/spec coverage
  docs, and `docs/DECODER-ROADMAP.md`.
- Public APIs and crate dependencies are unchanged.
- The minimal runtime decode path and committed fixture output are unchanged.
