## Context

`DECODE-COEFF-ALL-ZERO-BLOCK-STATE` models the § 5.20.7.27 all-zero branch
state effects. `DECODE-COEFF-EOB-DERIVED-SYMBOL-READ` reads the nonzero EOB
syntax from caller-resolved transform log2 dimensions and plane/inter facts. The
next coefficient-loop boundary should represent the spec branch point after the
`all_zero` symbol has been decoded: all-zero blocks update coefficient context
state, while nonzero blocks consume EOB syntax before future scan traversal.

## Goals / Non-Goals

**Goals:**
- Add Feature ID `DECODE-COEFF-EOB-BRANCH-HANDOFF`.
- Add a crate-private branch input/result pair for all-zero and nonzero EOB
  branch outcomes.
- Route the all-zero branch through `apply_all_zero_coeff_block`.
- Route the nonzero branch through `read_nonzero_coeff_eob_from_context`.
- Preserve no-consumption behavior for all-zero branch CDF/symbol state and
  invalid nonzero transform facts.
- Use the new handoff in the minimal flat-intra block-symbol trace's existing
  all-zero branches.

**Non-Goals:**
- No new syntax source for `all_zero`; the caller still decodes/asserts that
  symbol.
- No transform-size table lookup from real `txSz`, scan traversal, base/br/sign
  symbol reads, `Level[]` or `Quant[]` nonzero writes, `read_quant`,
  dequantization, inverse transform, residual add, or reconstruction.
- No public API, CLI, dependency graph, encoder, AVM/dav2d wrapper, or
  diagnostic changes.

## Decisions

1. Use an enum input rather than a boolean plus unused branch data.

   The future caller already knows which branch the decoded `all_zero` symbol
   selected. `CoeffBlockEobBranchInput::{AllZero, NonZero}` avoids requiring a
   dummy nonzero context when runtime code exercises only an all-zero trace.

2. Keep the handoff transactional at the helper level.

   The all-zero path only receives the coefficient context state mutation it
   needs; it does not consume CDF rows or symbol bits. The nonzero path derives
   selector facts before consuming mutable CDF or symbol state and does not touch
   coefficient context state.

3. Wire only the existing minimal all-zero trace.

   The runtime trace already has decoded luma and V `all_zero` symbols and
   applies all-zero state effects. Routing those applications through the handoff
   proves a production-shaped call site without introducing unsupported nonzero
   block syntax or changing fixture output.

## Risks / Trade-offs

- The nonzero branch remains helper-only until transform-block facts and scan
  traversal exist; matrix/support notes keep that partial status explicit.
- Passing mutable CDF/symbol references to an all-zero branch can look broader
  than needed; tests lock down that the branch leaves those states untouched.
