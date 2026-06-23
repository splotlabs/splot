## Why

The FSC/IDTX coefficient path now produces local `QuantSign[]`, signed
`Quant[]`, `culLevel`, and `dcCategory`, but it still stops before the
end-of-`coeffs()` tile context-line update. Adding the FSC counterpart to the
ordinary context-commit wrapper removes that asymmetry before runtime `coeffs()`
integration.

Feature ID: `DECODE-COEFF-FSC-CONTEXT-COMMIT`.

## What Changes

- Add a crate-private FSC context-commit wrapper after
  `DECODE-COEFF-FSC-QUANT-PASS`.
- Reuse `TileCoeffContextState::update_after_coeffs` with caller-resolved
  plane and 4x4 block geometry, committing final FSC `culLevel` and
  `dcCategory`.
- Preserve fail-atomic context behavior when the FSC quant pass fails or when
  the context update rejects caller facts.
- Add focused tests for successful above/left level and DC context writes plus
  pass-failure and update-failure rollback.
- Update OpenSpec, implementation/support/conformance tracking, roadmap, and
  generated status docs.

## Capabilities

### New Capabilities

- `coeff-fsc-context-commit`: loaded-but-unwired FSC/IDTX coefficient pass
  end-of-`coeffs()` tile context commit.

### Modified Capabilities

- `decoder-support`: track `DECODE-COEFF-FSC-CONTEXT-COMMIT` as partial decoder
  support.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/`, focused
  unit tests, decoder tracking docs, and OpenSpec change artifacts.
- Public APIs: none.
- Dependencies: none.
- Non-goals: runtime `coeffs()` wiring, deriving `useFsc`/`segEob` or block
  geometry from runtime syntax, dequantization, inverse transform, residual add,
  reconstruction/output, reference refresh, inter prediction, filters, external
  decoder invocation, and broad `decode_tile()` support.
