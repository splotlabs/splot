## Why

The nonzero coefficient EOB symbol reader now consumes caller-selected `eob_pt_*`
CDF rows, but its caller facts are still raw inputs. The next decoder brick should
derive the AV2 § 5.20.7.27 transform-size EOB class and `eobCtx` locally so the
future coefficient loop can choose the correct EOB CDF bank without importing
reconstruction types or inventing syntax behavior.

## What Changes

- Add Feature ID `DECODE-COEFF-EOB-SIZE-CONTEXT` to the implementation matrix.
- Add a crate-private `splot-decode` helper that maps caller-resolved
  `Tx_Width_Log2[txSz]` / `Tx_Height_Log2[txSz]` values to `EobPtSize` using
  `eobMultisize = Min(width_log2, 5) + Min(height_log2, 5) - 4`.
- Add a crate-private helper for `eobCtx = (plane > 0) ? 2 : is_inter`, preserving
  the `coeff_cdf_q_ctx` handoff into `read_nonzero_coeff_eob`.
- Add focused unit tests and support/coverage rows proving the size mapping, luma
  and chroma contexts, invalid log2 rejection, and integration with the existing
  symbol-reader input type.
- Do not wire the helper into a runtime coefficient loop yet.

## Capabilities

### New Capabilities
- `coeff-eob-size-context`: Derives the nonzero coefficient EOB transform-size
  class and `eobCtx` handoff facts for the existing EOB symbol reader.

### Modified Capabilities
- None.

## Impact

Affected code is limited to crate-private decode coefficient-loop helpers, their
tests, feature/support/coverage documentation, and this OpenSpec change. There
are no public API changes, new dependencies, dependency-graph changes, encoder
changes, diagnostics changes, or user-facing decode output claims. Broad AV2
coefficient scan walking, base/br/sign symbol reads, nonzero `Level[]` or
`Quant[]` writes, dequantization, inverse transforms, residual add,
reconstruction, AVM/dav2d invocation, and full `decode_block()` / `decode_tile()`
support remain non-goals for this brick.
