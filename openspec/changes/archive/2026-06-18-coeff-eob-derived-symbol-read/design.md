## Context

`DECODE-COEFF-EOB-SYMBOL-READ` reads the nonzero § 5.20.7.27 EOB syntax once
the caller has selected `EobPtSize`, `coeff_cdf_q_ctx`, and `eobCtx`.
`DECODE-COEFF-EOB-SIZE-CONTEXT` derives those selector facts from
caller-resolved transform log2 dimensions and plane/inter state. The future
coefficient loop should call one crate-private boundary that performs both steps
in the same order as the spec: derive selector facts, then read the active
`eob_pt_*` syntax.

## Goals / Non-Goals

**Goals:**
- Add Feature ID `DECODE-COEFF-EOB-DERIVED-SYMBOL-READ`.
- Add a crate-private helper that accepts `NonZeroCoeffEobContextInput` and calls
  `read_nonzero_coeff_eob` after deriving `NonZeroCoeffEobSymbolInput`.
- Preserve typed error propagation for invalid transform facts and symbol/literal
  read failures.
- Prove invalid transform log2 inputs do not consume CDF rows or symbol bits.

**Non-Goals:**
- No runtime `coeffs()` loop wiring.
- No transform-block syntax lookup, scan traversal, base/br/sign symbol reads,
  `Level[]` or `Quant[]` writes, `read_quant`, dequantization, inverse transform,
  residual add, or reconstruction.
- No public API, CLI, dependency graph, encoder, AVM/dav2d wrapper, or diagnostic
  changes.

## Decisions

1. Compose the existing helpers instead of merging their responsibilities.

   The derived reader should be a thin boundary over
   `nonzero_coeff_eob_symbol_input` and `read_nonzero_coeff_eob`. Keeping the
   selector derivation and actual symbol read separately lets tests continue to
   cover each layer while providing the future runtime loop a single call site.

2. Derive before touching mutable state.

   The helper derives and validates `NonZeroCoeffEobSymbolInput` before borrowing
   mutable CDF or symbol-decoder state for symbol reads. That makes invalid
   transform log2 dimensions fail without row mutation or arithmetic-decoder
   consumption.

3. Keep status partial.

   This change reads EOB syntax from caller-resolved facts but still has no
   runtime transform-block source for those facts and no coefficient scan or
   quantization writes. The implementation and support rows must remain partial.

## Risks / Trade-offs

- The helper is still loaded ahead of the real runtime loop -> matrix/support
  notes keep this explicit and tests focus on the handoff contract.
- A wrapper can look too small to be a separate feature -> this boundary is the
  first production-shaped call that combines EOB context derivation with CDF
  consumption, which is exactly the next prerequisite for the coefficient loop.
