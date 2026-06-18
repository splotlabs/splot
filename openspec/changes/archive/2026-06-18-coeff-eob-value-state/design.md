## Context

`DECODE-COEFF-ALL-ZERO-BLOCK-STATE` gives `splot-decode` a checked all-zero
branch for AV2 § 5.20.7.27 `coeffs()`: zero local `Level[]`, `QuantSign[]`,
and `Quant[]` state plus zero level/DC context-line writes. The next nonzero
branch starts by deriving `eob` from an already decoded `eobPt`, optional
`eob_extra`, and the following `eob_extra_bit` refinement bits. The tile CDF
subset already owns the `eob_pt_*` and `eob_extra` rows, but the decode loop
does not consume them yet.

## Goals / Non-Goals

**Goals:**

- Add a narrow, checked coefficient-loop value helper for the nonzero EOB
  arithmetic in AV2 § 5.20.7.27.
- Make invalid caller-provided EOB parts typed errors, not panics or silent
  saturation.
- Record the new partial row in decoder-support and implementation tracking.

**Non-Goals:**

- No new `SymbolDecoder::read_symbol` calls for `eob_pt_*` or `eob_extra`.
- No transform-size dispatch, scan-order traversal, `Level[]` writes, `Quant[]`
  writes, `read_quant`, dequantization, transform, residual add, or output
  change.
- No new public API, crate dependency, AVM/dav2d invocation, or CI dependency.

## Decisions

1. Keep the helper caller-resolved.

   The helper accepts `eobPt`, `eob_extra`, and packed `eob_extra_bit`
   refinement bits after the caller has chosen the `eob_pt_*` bank and decoded
   any size-specific `eob_pt_*_extra` syntax. This avoids mixing CDF row
   selection, literal bit reads, and value arithmetic in one change. The next
   consumer can wire actual symbol reads around this value helper.

2. Treat unreachable combinations as typed errors.

   `eobPt == 0`, oversized `eobPt`, refinements supplied for `eobPt < 3`, and
   packed refinement bits outside the bit width implied by `eobPt` should return
   crate-private `CoeffLoopContextError` variants. That keeps malformed
   intermediate state visible during later wiring.

3. Do not mutate coefficient block state in this change.

   EOB calculation is a prerequisite to scan walking and coefficient writes, but
   it does not by itself know the scan order or coefficient levels. Keeping the
   helper value-only prevents overclaiming output behavior and makes the tests
   self-contained.

## Risks / Trade-offs

- Caller-resolved inputs can be miswired later -> typed validation catches
  impossible EOB parts before any state mutation.
- The helper is loaded before production consumption -> focused tests and matrix
  notes must be explicit that decode output is unchanged.
- EOB arithmetic uses shifts -> bounds are constrained to the AV2 EOB point
  range before shifting.
