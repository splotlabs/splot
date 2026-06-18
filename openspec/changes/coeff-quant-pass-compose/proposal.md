## Why

The ordinary non-FSC coefficient path now has separate helpers for checked scan
walking, local `Level[]` writes, sign reads, AV2 § 5.20.7.28 `read_quant`
literal parsing, and local `Quant[]` writes. The next narrow decoder step is to
compose the `read_quant` parser with the quant-state writer so one crate-private
boundary can perform the second ordinary non-FSC coefficient pass without yet
deriving real runtime selectors, scan tables, or reconstruction inputs.

## What Changes

- Add Feature ID `DECODE-COEFF-QUANT-PASS-COMPOSE`.
- Add a crate-private `splot-decode` helper that accepts checked scan entries,
  local coefficient state, sign summaries, caller-derived `maxLevel` facts, and
  block-level hidden, TCQ, and lossless facts.
- Preflight all caller facts before consuming any `read_quant` literal bits.
- Call the existing § 5.20.7.28 `read_quant` parser and feed its
  `CoeffQuantReadInput` records into the existing quant-state writer.
- Keep runtime `coeffs()` integration, selector derivation, dequantization,
  inverse transforms, residual add, reconstruction, and output bytes unchanged.

## Capabilities

### New Capabilities

- `coeff-quant-pass-compose`: crate-private ordinary non-FSC composition of
  `read_quant` syntax parsing and `Quant[]` state writes.

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
- Public APIs and crate dependencies are unchanged.
- The minimal runtime decode path and committed fixture output are unchanged.
