## Context

`DECODE-COEFF-EOB-SYMBOL-READ` added a crate-private reader for the nonzero
§ 5.20.7.27 EOB syntax, but it intentionally requires the caller to provide the
active `EobPtSize` and `eobCtx`. The AV2 spec derives those facts immediately
before the `eob_pt_*` symbol read:

- `eobMultisize = Min(Tx_Width_Log2[txSz], 5) + Min(Tx_Height_Log2[txSz], 5) - 4`
- `eobCtx = (plane > 0) ? 2 : is_inter`

The coefficient-loop module already owns the EOB value and EOB symbol-read
composition. This change fills the missing transform-size/context handoff without
claiming runtime coefficient-loop support.

## Goals / Non-Goals

**Goals:**
- Add Feature ID `DECODE-COEFF-EOB-SIZE-CONTEXT`.
- Derive the seven AV2 EOB-point CDF families (`Pt16` through `Pt1024`) from
  caller-resolved transform log2 dimensions.
- Derive `eobCtx` for luma intra, luma inter, and chroma planes.
- Preserve `coeff_cdf_q_ctx` and produce the existing `NonZeroCoeffEobSymbolInput`
  for `read_nonzero_coeff_eob`.
- Reject invalid transform log2 dimensions before underflow or silent remapping.

**Non-Goals:**
- No runtime `coeffs()` loop wiring.
- No coefficient scan traversal, base/br/sign symbol reads, `Level[]` or `Quant[]`
  writes, `read_quant`, dequantization, inverse transform, residual add, or
  reconstruction.
- No public API, CLI, dependency graph, encoder, AVM/dav2d wrapper, or diagnostic
  changes.

## Decisions

1. Keep the input scalar and crate-private.

   The helper accepts `tx_width_log2`, `tx_height_log2`, `plane`, `is_inter`, and
   `coeff_cdf_q_ctx` rather than a transform-size enum. This matches the spec
   values consumed by § 5.20.7.27 and avoids coupling the entropy handoff to
   reconstruction-side transform-size or transform-class types. The alternative
   was to import a richer transform descriptor, but that would broaden the
   dependency surface before the runtime coefficient loop needs it.

2. Fail on log2 dimensions below the AV2 transform minimum.

   Valid transform sizes have width and height log2 values at least 2 for 4x4
   samples. Since the spec expression subtracts 4, accepting smaller values would
   either underflow or require a non-spec saturating behavior. The helper returns a
   typed `CoeffLoopContextError` for either log2 value below 2. Larger values are
   clamped with `Min(..., 5)` exactly as in § 5.20.7.27.

3. Return the existing symbol-reader input type.

   `read_nonzero_coeff_eob` already owns the actual CDF row reads and literal
   refinement parsing. A small constructor-style helper can produce
   `NonZeroCoeffEobSymbolInput`, keeping the boundary explicit and testable.

## Risks / Trade-offs

- Invalid caller facts could still be provided by future runtime code -> focused
  unit tests cover invalid log2 rejection and preserve typed errors.
- The helper still does not prove a real `txSz` table lookup -> the implementation
  matrix and support rows remain partial, with explicit notes that runtime
  transform-block syntax wiring is deferred.
- Chroma uses `plane > 0` rather than validating the plane range -> this follows
  the spec expression and leaves stricter plane validation to callers that own
  plane syntax/state.
