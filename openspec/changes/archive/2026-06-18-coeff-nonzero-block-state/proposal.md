## Why

The coefficient EOB branch handoff can now dispatch to the nonzero EOB reader,
but the nonzero branch still returns only the EOB syntax result. The future scan
walk needs the same local coefficient arrays that the all-zero branch already
allocates: zero-initialized `Level[]`, `QuantSign[]`, and `Quant[]` sized from
caller-resolved transform geometry. Adding that nonzero block-start shell is the
next small prerequisite before coefficient base/br/sign symbol reads can fill
`Quant[]`.

## What Changes

- Add Feature ID `DECODE-COEFF-NONZERO-BLOCK-STATE` to the implementation
  matrix.
- Add a crate-private nonzero coefficient block-start input/result that carries
  caller-resolved block geometry plus the existing nonzero EOB context facts.
- Allocate the local transform coefficient block state before consuming nonzero
  EOB symbols so invalid transform extents fail without CDF or symbol-decoder
  consumption.
- Update the EOB branch handoff nonzero arm to return the block-start shell
  containing both zeroed local coefficient state and the EOB read result.
- Move the branch handoff into a small child module to keep `coeff_loop.rs` under
  the repository source-line budget.
- Add focused tests for nonzero block allocation, invalid geometry
  no-consumption behavior, and preservation of all previous branch contracts.

## Capabilities

### New Capabilities
- `coeff-nonzero-block-state`: Initializes local nonzero coefficient-block state
  before reading the nonzero EOB syntax, producing the block container future
  scan traversal will populate.

### Modified Capabilities
- `coeff-eob-branch-handoff`: Its nonzero branch result now includes the local
  zeroed coefficient block state alongside the EOB read result.

## Impact

Affected code is limited to crate-private decode coefficient-loop helpers, tests,
feature/support/coverage documentation, and this OpenSpec change. There are no
public API changes, new dependencies, dependency-graph changes, encoder changes,
diagnostics changes, or decode output changes. Scan traversal, coefficient
base/br/sign symbol reads, nonzero `Level[]` or `Quant[]` writes, `read_quant`,
dequantization, inverse transforms, residual add, reconstruction, AVM/dav2d
invocation, and full `decode_block()` / `decode_tile()` support remain non-goals
for this brick.
