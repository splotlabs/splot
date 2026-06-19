## Why

The FSC/IDTX coefficient path now derives and consumes level and sign symbols, but
it still stops before the second-loop `read_quant` step that produces local
`Quant[]` values. Adding this loaded-but-unwired pass closes the remaining local
FSC coefficient-state gap before runtime `coeffs()` integration.

Feature ID: `DECODE-COEFF-FSC-QUANT-PASS`.

## What Changes

- Add a crate-private FSC/IDTX sign/quant composition after
  `DECODE-COEFF-FSC-LEVEL-PASS`, reusing the sign derivation/read step from
  `DECODE-COEFF-FSC-SIGN-PASS`.
- Reuse the existing §5.20.7.28 `read_quant` state machine with FSC facts:
  hidden parity disabled, TCQ disabled, and `maxLevel =
  NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1`.
- Interleave `idtx_sign`, immediate `QuantSign[]` writes, `read_quant`, signed
  `Quant[pos]` writes, and final `culLevel` / `dcCategory` derivation in spec
  order.
- Add focused tests for quant reads, signed quant writes, zero-level behavior,
  sign/quant ordering, DC category, and static no-consumption/no-mutation
  failures.
- Update OpenSpec, implementation/support/conformance tracking, roadmap, and
  generated status docs.

## Capabilities

### New Capabilities

- `coeff-fsc-quant-pass`: loaded-but-unwired FSC/IDTX second-loop `read_quant`
  and signed `Quant[]` state writes.

### Modified Capabilities

- `decoder-support`: track `DECODE-COEFF-FSC-QUANT-PASS` as partial decoder
  support.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/`, coefficient
  state helpers, focused unit tests, decoder tracking docs, and OpenSpec change
  artifacts.
- Public APIs: none.
- Dependencies: none.
- Non-goals: runtime `coeffs()` wiring, tile context commit for the FSC path,
  dequantization, inverse transform, residual add, reconstruction/output,
  reference refresh, inter prediction, filters, external decoder invocation, and
  broad `decode_tile()` support.
