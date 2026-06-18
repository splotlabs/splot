## Why

The decoder now has separate coefficient EOB foundations: the all-zero branch
state application and the nonzero EOB reader from caller-resolved transform
facts. A future `coeffs()` loop needs one crate-private handoff after the
decoded `all_zero` symbol to dispatch to the correct branch without duplicating
state or EOB selector wiring.

## What Changes

- Add Feature ID `DECODE-COEFF-EOB-BRANCH-HANDOFF` to the implementation matrix.
- Add a crate-private coefficient EOB branch handoff that accepts either an
  all-zero block input or a nonzero EOB context input.
- Dispatch the all-zero branch to the existing all-zero coefficient-block state
  application and the nonzero branch to the existing derived EOB reader.
- Route the minimal flat-intra block-symbol trace's luma and V all-zero state
  applications through the new handoff while preserving its current output.
- Add focused tests for all-zero state application without symbol/CDF
  consumption, nonzero derived EOB reading without coefficient context mutation,
  and invalid nonzero transform facts preserving all mutable state.

## Capabilities

### New Capabilities
- `coeff-eob-branch-handoff`: Dispatches the coefficient-loop EOB path after the
  decoded `all_zero` decision to either all-zero coefficient state application or
  nonzero derived EOB symbol reading.

### Modified Capabilities
- None.

## Impact

Affected code is limited to crate-private decode coefficient-loop helpers, the
minimal block-symbol trace's all-zero call sites, tests, feature/support/coverage
documentation, and this OpenSpec change. There are no public API changes, new
dependencies, dependency-graph changes, encoder changes, diagnostics changes, or
new user-facing decode output claims. Broad transform-block syntax, scan walking,
base/br/sign coefficient symbol reads, nonzero `Level[]` or `Quant[]` writes,
`read_quant`, dequantization, inverse transforms, residual add, reconstruction,
AVM/dav2d invocation, and full `decode_block()` / `decode_tile()` support remain
non-goals for this brick.
