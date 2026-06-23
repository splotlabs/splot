## Why

The coefficient CDF subset already covers ordinary non-IDTX base/base-EOB/base-range
rows plus the parity-hidden Ph row, while the generated AV2 defaults and §8.3.2
selectors also define the FSC/IDTX row families used by `coeffs()` when
`useFsc` is active. Loading those rows is the next narrow decoder-conformance
brick before implementing the actual FSC/IDTX symbol pass.

Feature ID: `DECODE-COEFF-IDTX-CDF-ROWS`.

## What Changes

- Add crate-private `TileCoeffBaseBobCdf`, `TileCoeffBaseIdtxCdf`,
  `TileCoeffBrIdtxCdf`, and `TileIdtxSignCdf` storage to the coefficient CDF
  row subset from generated AV2 v1.0.0 §9.3 defaults.
- Add typed immutable and mutable `CoeffCdfSelector` variants with q-context,
  `Min(TX_16X16, txSzCtx)` context, and symbol-context bounds errors.
- Include the rows in tile copy/save/average and frame-end count scaling.
- Add focused tests for generated default selection, invalid selector axes,
  tile-copy isolation, lifecycle averaging/scaling, and mutable symbol-reader
  handoff.
- Update decoder tracking docs, generated status, and OpenSpec artifacts.

## Capabilities

### New Capabilities

- `coeff-idtx-cdf-rows`: FSC/IDTX coefficient CDF row loading, selection, and
  lifecycle handling for the crate-private tile CDF subset.

### Modified Capabilities

- `decoder-support`: record the new partial decoder boundary and clarify that
  runtime `coeffs()` FSC/IDTX integration remains unsupported.

## Impact

Affected code is limited to `splot-decode` coefficient CDF row storage and tests.
There are no public API, dependency, licensing, encoder, or CLI changes. Decode
output for the current minimal fixture remains unchanged because runtime nonzero
coefficient blocks still do not call a FSC/IDTX coefficient symbol loop.
