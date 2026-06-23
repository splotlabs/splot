## Context

`DECODE-COEFF-IDTX-CDF-ROWS` loads the FSC/IDTX coefficient CDF rows and
`DECODE-COEFF-FSC-SCAN-WALK` derives the checked `bob..segEob` scan window from
AV2 section 5.20.7.27. The remaining first-pass gap is symbol sequencing and
local `Level[]` mutation for:

- `c == bob`: `coeff_base_bob`, then `level = coeff_base_bob + 1`
- later entries: `coeff_base_idtx`, then `level = coeff_base_idtx`
- every entry: if `level > NUM_BASE_LEVELS`, read `coeff_br_idtx` and add it
- write `Level[row][col] = level`

## Goals / Non-Goals

**Goals:**

- Add a separate FSC/IDTX level-pass module and result type.
- Validate scan cardinality, block geometry, and row-major entry consistency
  before consuming CDF rows.
- Derive selectors using the existing section 8.3.2 context helpers:
  `coeff_base_bob_ctx`, `coeff_base_idtx_ctx`, and `coeff_br_idtx_ctx`.
- Clamp the selector transform-size axis to the FSC row domain
  `Min(TX_16X16, txSzCtx)`.
- Preserve no runtime output change.

**Non-Goals:**

- No `idtx_sign`, `read_quant`, `QuantSign[]`, `Quant[]`, tile context commit,
  dequantization, inverse transform, residual add, reconstruction, output, or
  reference refresh.
- No `useFsc` derivation, `segEob` derivation from `txSz`, runtime `coeffs()`
  integration, public API, CLI, dependency, licensing, encoder, AVM, or dav2d
  change.

## Decisions

1. **Separate pass type.**
   The FSC path is forward and IDTX-specific, so it gets
   `NonZeroCoeffFscLevelPass` rather than overloading the ordinary non-FSC
   base/level pass.

2. **Preflight before reads.**
   Static facts are checked before CDF or symbol mutation. Dynamic selector
   errors can still occur when the selected row is first reached, matching the
   existing ordinary pass behavior.

3. **Clamp only the FSC tx-size row axis.**
   The spec row index is `Min(TX_16X16, txSzCtx)`. The helper accepts the
   caller-resolved `txSzCtx` and performs the clamp locally so callers cannot
   pass an unclamped but legal larger transform-size context into a row selector
   that only owns the three FSC row families.

## Risks / Trade-offs

- **Still loaded-but-unwired** -> Tracking and docs must keep runtime
  `coeffs()` and decoded output unsupported.
- **Symbol payload tests** -> Unit tests search small payloads instead of
  depending on a brittle hand-authored CDF bit pattern.
- **No rollback after partial symbol reads** -> This helper is a staged parser
  boundary like the ordinary pass. It validates static facts before reads, but a
  symbol error after earlier reads can leave CDF/symbol state advanced; the
  future runtime caller owns transaction boundaries around broader block decode.
