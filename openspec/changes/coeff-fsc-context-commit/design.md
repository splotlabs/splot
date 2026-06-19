## Context

`DECODE-COEFF-FSC-QUANT-PASS` implements the loaded-but-unwired FSC/IDTX
second loop in AV2 §5.20.7.27: it interleaves `idtx_sign`, `read_quant`, local
`QuantSign[]` writes, local signed `Quant[]` writes, and final `culLevel` /
`dcCategory` derivation. The ordinary non-FSC path already wraps its equivalent
pass with an end-of-`coeffs()` context commit through
`TileCoeffContextState::update_after_coeffs`.

The FSC path should commit the same tile context lines after the local FSC pass
succeeds. That commit is a block-level side effect of `coeffs()`, not a
reconstruction step, and can remain crate-private and loaded-but-unwired until
the runtime tile decode loop can supply real `useFsc`, `segEob`, transform, and
geometry facts.

## Goals / Non-Goals

**Goals:**

- Add a crate-private `apply_nonzero_coeff_fsc_quant_pass_with_context_commit`
  wrapper.
- Commit final `culLevel` and `dcCategory` from `NonZeroCoeffFscQuantPass` to
  `AboveLevelContext`, `LeftLevelContext`, `AboveDcContext`, and
  `LeftDcContext` via the existing checked tile-state helper.
- Keep context updates fail-atomic on both pass failure and invalid
  caller-resolved update facts.
- Keep the helper loaded-but-unwired and fully private to `splot-decode`.

**Non-Goals:**

- Runtime `coeffs()` wiring.
- Deriving `useFsc`, `segEob`, `txSz`, scan order, plane, or block geometry from
  runtime syntax.
- Dequantization, inverse transform, residual add, reconstruction/output, or
  reference refresh.
- New dependencies, public APIs, AVM/dav2d integration, or broad `decode_tile()`
  support.

## Decisions

- Reuse `TileCoeffContextState::update_after_coeffs` instead of adding an
  FSC-specific context writer. The state update is identical once `culLevel`,
  `dcCategory`, plane, and 4x4 geometry are known.
- Accept caller-resolved context commit facts in a small config struct, matching
  the ordinary path. Deriving those facts belongs to later runtime
  `coeffs()` wrappers.
- Run the FSC quant pass before updating tile context lines. This preserves the
  AV2 §5.20.7.27 ordering and keeps failed symbol reads from mutating tile
  context state.

## Risks / Trade-offs

- The wrapper still depends on staged caller facts for plane and 4x4 geometry.
  That is intentional for this brick; those facts remain tracked as runtime
  integration work.
- If context update validation fails after a successful symbol pass, the symbol
  decoder and tile CDFs may already be advanced. This mirrors existing ordinary
  staged behavior; the tile context state itself remains unchanged on update
  failure.
