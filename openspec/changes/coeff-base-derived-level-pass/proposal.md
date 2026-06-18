## Why

The ordinary non-FSC coefficient composer still requires every base/base-range
CDF selector up front, but AV2 derives later `coeff_base` and `coeff_br` rows
from `Level[]` values that are produced earlier in the same first pass. The next
narrow decoder step is a loaded, unwired first-pass helper that reads one base
symbol, writes `Level[]`, updates first-pass state, and then derives the next
selector from the evolving local block.

## What Changes

- Add Feature ID `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS`.
- Add a crate-private `splot-decode` ordinary non-FSC first-pass helper that
  starts from `NonZeroCoeffBlockStart` plus a checked scan walk and derives
  `coeff_base_eob`, `coeff_base`, and conditional `coeff_br` selectors while
  local `Level[]` evolves.
- Reuse the existing § 8.3.2 coefficient context derivations and base symbol
  reader primitives, and write each decoded level immediately before processing
  the next checked scan entry.
- Track first-pass `tcqState`, `sumAbs1`, `numNz`, and derived `isHidden` for
  future second-pass sign/quant integration.
- Keep the parity-hidden-only `TileCoeffBasePhCdf` row unsupported if selector
  derivation reaches it; that row remains a separate future CDF-loading step.
- Keep runtime `coeffs()` integration, scan-table derivation, transform-type
  derivation, sign-source derivation, tile context commits, dequantization,
  inverse transforms, residual add, reconstruction, and output bytes unchanged.

## Capabilities

### New Capabilities

- `coeff-base-derived-level-pass`: state-derived ordinary non-FSC base/base-range
  first pass from nonzero block start through local `Level[]` writes and
  first-pass parity/TCQ summary.

### Modified Capabilities

- `decoder-support`: records the new decoder-support row and keeps broad
  coefficient decode/reconstruction rows partial while runtime wiring remains
  out of scope.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/`,
  `crates/splot-decode/src/tile_payload/cdf/coeff_context.rs`, and focused
  coefficient-loop tests.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated feature/support/spec coverage
  docs, and `docs/DECODER-ROADMAP.md`.
- Public APIs, crate dependencies, and the minimal runtime decode path are
  unchanged.
