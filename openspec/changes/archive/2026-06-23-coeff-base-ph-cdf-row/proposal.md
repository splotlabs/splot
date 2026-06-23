## Why

The state-derived coefficient base/level pass can now reach the parity-hidden
DC `coeff_base` selector, but the decode CDF subset still omits the
`TileCoeffBasePhCdf` row and fails closed at that point. Loading and selecting
that row is the next narrow decoder-conformance brick before composing more of
the runtime nonzero coefficient path.

Feature ID: `DECODE-COEFF-BASE-PH-CDF-ROW`.

## What Changes

- Add crate-private `TileCoeffBasePhCdf` storage to the ordinary coefficient CDF
  subset from the generated AV2 v1.0.0 §9.3 defaults.
- Add typed `CoeffCdfSelector::BasePh` immutable and mutable row selection with
  bounds errors, tile copy/average coverage, and frame-end count scaling.
- Map `CoeffBaseSelection::Ph` in the derived base/level first pass to the
  loaded parity-hidden base row instead of returning an unsupported selector
  error.
- Add eob>=5 hidden-parity coverage proving the first pass reaches and consumes
  `TileCoeffBasePhCdf`.
- Update decoder tracking docs, generated status, and OpenSpec artifacts.

## Capabilities

### New Capabilities

- `coeff-base-ph-cdf-row`: parity-hidden coefficient base CDF row loading,
  selection, lifecycle handling, and first-pass consumption.

### Modified Capabilities

- `decoder-support`: record the new partial decoder boundary and clarify that
  runtime `coeffs()` integration remains unsupported.

## Impact

Affected code is limited to `splot-decode` coefficient CDF row storage/tests and
the loaded-but-unwired ordinary non-FSC base/level first-pass helper. There are
no public API, dependency, licensing, encoder, or CLI changes. Decode output for
the current minimal fixture remains unchanged because runtime nonzero
coefficient blocks still do not call this helper.
