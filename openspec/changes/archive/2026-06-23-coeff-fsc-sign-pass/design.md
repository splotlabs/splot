## Context

`DECODE-COEFF-FSC-LEVEL-PASS` consumes the first `useFsc` loop in AV2 section
5.20.7.27 and leaves the local coefficient block with `Level[]` populated for
the checked `bob..segEob` window. The next FSC loop iterates `c = 0..segEob`,
reads `idtx_sign` only when `Level[row][col] != 0`, then eventually calls
`read_quant` and writes `Quant[]` and `QuantSign[]`.

The sign context in AV2 section 8.3.2 is stateful: `idtx_sign_ctx` reads left,
above, and above-left `QuantSign[]` neighbours. Therefore the staged sign pass
must update local `QuantSign[]` as it reads each nonzero sign, even though
`read_quant` and `Quant[]` writes remain deferred.

## Goals / Non-Goals

**Goals:**

- Add a separate FSC/IDTX sign-pass module and result type.
- Walk all `0..segEob` scan entries from the caller-provided scan table.
- Derive `IdtxSign` selectors from current local `QuantSign[]`, `Level[]`, q
  context, and clamped `Min(TX_16X16, txSzCtx)`.
- Read `idtx_sign` only for nonzero `Level[]` entries and write local
  `QuantSign[row][col] = -1` or `1` after each read.
- Preserve no runtime output change.

**Non-Goals:**

- No `read_quant`, nonzero `Quant[]`, `dcCategory`/`culLevel` updates, tile
  context commit, dequantization, inverse transform, residual add,
  reconstruction, output, or reference refresh.
- No `useFsc` derivation, `segEob` derivation from `txSz`, runtime `coeffs()`
  integration, public API, CLI, dependency, licensing, encoder, AVM, or dav2d
  change.

## Decisions

1. **Separate FSC sign pass.**
   The ordinary sign helper handles `dc_sign`, `dc_sign_horz_vert`, and raw
   `sign_bit`. FSC/IDTX uses only `idtx_sign` plus an evolving `QuantSign[]`
   context, so a dedicated helper keeps the two paths explicit.

2. **Own and mutate the level-pass block.**
   The sign pass consumes `NonZeroCoeffFscLevelPass`, carries forward its EOB
   and level-read evidence, and mutates the owned local block. This avoids
   pretending the sign pass can be a stateless read-only phase.

3. **Preflight static facts before reads.**
   Geometry, scan length, row-major positions, and state bounds are validated
   before any `idtx_sign` row is consumed. Dynamic symbol failures after earlier
   reads keep the same staged-parser contract as the ordinary coefficient pass:
   a future runtime caller owns larger transaction checkpoints.

4. **Keep Quant[] untouched.**
   `QuantSign[]` is updated because later `idtx_sign` contexts require it.
   `Quant[]` remains zero because the spec does not produce quantized
   coefficients until the deferred `read_quant` step.

## Risks / Trade-offs

- **Partial mutation on late symbol failure** -> Static invalid facts fail
  atomically before reads; later runtime integration will own broader rollback
  around full block decode.
- **Staged order differs from final `read_quant` placement** -> Early
  `QuantSign[]` writes are equivalent for sign-context evolution because
  `read_quant` does not feed `idtx_sign_ctx`; tests pin the visible context
  dependency.
- **Still loaded-but-unwired** -> Tracking and docs must keep runtime
  `coeffs()` and decoded output unsupported.
