## Why

The decoder now has both pieces of the nonzero coefficient EOB path: a symbol
reader that consumes caller-selected EOB CDF rows, and a helper that derives those
selection facts from transform log2 dimensions and plane/inter state. The next
small step is to compose those pieces behind one crate-private entry point so a
future coefficient loop can read EOB syntax from caller-resolved transform facts
without duplicating selection logic.

## What Changes

- Add Feature ID `DECODE-COEFF-EOB-DERIVED-SYMBOL-READ` to the implementation
  matrix.
- Add a crate-private `splot-decode` helper that accepts
  `NonZeroCoeffEobContextInput`, derives `NonZeroCoeffEobSymbolInput`, and then
  calls the existing `read_nonzero_coeff_eob`.
- Preserve the existing typed error behavior: invalid transform log2 dimensions
  fail before any CDF row or symbol state is consumed; symbol and literal read
  errors still propagate through the existing error variants.
- Add focused tests for direct-read equivalence and invalid-input no-consumption
  behavior.
- Do not wire the helper into the runtime coefficient loop yet.

## Capabilities

### New Capabilities
- `coeff-eob-derived-symbol-read`: Reads nonzero coefficient EOB syntax using
  caller-resolved transform and plane/inter facts, by composing the derived EOB
  context helper with the existing EOB symbol reader.

### Modified Capabilities
- None.

## Impact

Affected code is limited to crate-private decode coefficient-loop helpers, tests,
feature/support/coverage documentation, and this OpenSpec change. There are no
public API changes, new dependencies, dependency-graph changes, encoder changes,
diagnostics changes, or user-facing decode output claims. Broad AV2 coefficient
scan walking, base/br/sign symbol reads, nonzero `Level[]` or `Quant[]` writes,
`read_quant`, dequantization, inverse transforms, residual add, reconstruction,
AVM/dav2d invocation, and full `decode_block()` / `decode_tile()` support remain
non-goals for this brick.
