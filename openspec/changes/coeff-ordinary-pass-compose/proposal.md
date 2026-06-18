## Why

The ordinary non-FSC coefficient path now has separate helpers for nonzero EOB
start, checked scan walking, base/base-range reads, local `Level[]` writes, and
the later per-coefficient sign, max-level, `read_quant`, and signed `Quant[]`
steps. The next
narrow decoder step is to compose those existing pieces into one crate-private
ordinary pass boundary before deriving runtime selectors or committing tile
context lines.

## What Changes

- Add Feature ID `DECODE-COEFF-ORDINARY-PASS-COMPOSE`.
- Add a crate-private `splot-decode` helper that accepts a nonzero block start,
  caller-resolved scan table, caller-resolved base and sign read inputs, caller
  resolved plane/transform-class facts, and hidden/sumAbs1/TCQ/lossless
  quant-pass facts.
- Compose checked scan walking, base symbol reads, local `Level[]` writes, and
  the per-coefficient interleaved sign, derived-`maxLevel`, `read_quant`, and
  signed `Quant[]` write steps while resetting `hrLevelAvg` to 0 at block
  entry.
- Keep runtime `coeffs()` integration, evolving base selector derivation,
  post-level sign-source selection, tile context commits, dequantization,
  inverse transforms, residual add, reconstruction, and output bytes unchanged.

## Capabilities

### New Capabilities

- `coeff-ordinary-pass-compose`: crate-private ordinary non-FSC composition from
  nonzero EOB block start through signed `Quant[]` writes.

### Modified Capabilities

- `decoder-support`: records the new decoder-support row and keeps broad
  coefficient decode/reconstruction rows partial while runtime wiring remains
  out of scope.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/` and focused
  coefficient-loop tests.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated feature/support/spec coverage
  docs, and `docs/DECODER-ROADMAP.md`.
- Public APIs, crate dependencies, and the minimal runtime decode path are
  unchanged.
