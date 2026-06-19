## Context

The ordinary non-FSC coefficient branch now derives transform-size dimensions,
scan order, the non-lossless intra chroma non-directional `PlaneTxType` subset,
and AV2 §5.20.8.3 `txSet`. The remaining staged path still rejects every
lossless `compute_tx_type` input in the lower `Mode_To_Txfm` wrapper even though
§5.20.7.29 resolves several lossless cases before `get_tx_set(txSz, plane)`.

This brick covers only the lossless outcome that can be represented without
additional frame-state lookups: selecting `DCT_DCT` and then reusing the
existing transform-size-dimensions handoff. Broader lossless FSC/IDTX,
inter-luma/chroma state lookup, and runtime syntax dispatch stay unsupported.

## Goals / Non-Goals

**Goals:**

- Add a loaded-but-unwired ordinary branch wrapper above the existing `txSet`
  handoff for AV2 §5.20.7.29 lossless `DCT_DCT` selection.
- Preserve all-zero behavior without requiring lossless, `txSet`, `UVMode`, or
  reduced-transform-set facts.
- For nonzero `lossless == true && is_inter == false`, bypass non-lossless
  `get_tx_set` and `Mode_To_Txfm` validation, set `PlaneTxType = DCT_DCT`, and
  delegate to the existing transform-size-dimensions wrapper.
- Reject `lossless == true && is_inter == true` atomically because that subset
  needs the broader lossless IDTX / `TxTypes` branches.
- For nonzero `lossless == false`, delegate unchanged to
  `apply_coeff_ordinary_branch_from_tx_set`.
- Prove equivalence against lower explicit wrappers and prove fail-atomic
  handling of invalid transform-size domains.

**Non-Goals:**

- No full implementation of §5.20.7.29 `compute_tx_type`.
- No FSC/IDTX lossless branch, inter `TX_4X4` checks, chroma luma-transform
  lookup, luma `TxTypes` lookup, directional `wide_angle_mapping`, or runtime
  `coeffs()` wiring.
- No frame-header or block-syntax derivation of `Lossless`, `fsc_mode`,
  `FscModes`, `is_inter`, `reduced_tx_set`, `enable_chroma_dctonly`, `UVMode`,
  parity hiding, or TCQ facts.
- No output changes, dequantization, inverse transform, reconstruction,
  reference refresh, public API changes, or dependency graph changes.

## Decisions

1. Add a new wrapper above `txSet`.

   The spec evaluates `Lossless` before `get_tx_set`. Placing this wrapper above
   `apply_coeff_ordinary_branch_from_tx_set` preserves that order and avoids
   validating lower non-lossless facts when a staged lossless input has already
   resolved to `DCT_DCT`.

2. Reuse the transform-size-dimensions handoff for lossless `DCT_DCT`.

   The lower tx-size wrapper already validates generated transform-size tables,
   derives `txSzCtx`, derives scan order from `PlaneTxType`, and performs the
   fail-atomic transition into the ordinary pass. Feeding it `DCT_DCT` keeps the
   new layer small and testable.

3. Keep the lower `Mode_To_Txfm` lossless rejection intact.

   That wrapper still represents only the non-lossless intra chroma
   non-directional subset. The new lossless wrapper handles spec ordering above
   it without expanding the lower subset beyond its documented contract.

4. Keep caller-resolved lossless facts explicit.

   Runtime derivation of `Lossless` from segment state and quantizer state is
   separate frame/block integration work. The wrapper accepts the boolean so
   tests can prove ordering and behavior without inventing missing runtime
   state.

## Risks / Trade-offs

- Incorrectly overclaiming lossless support -> Mitigation: scope names, matrix
  residuals, support rows, and docs call out unsupported FSC/IDTX, luma/inter
  transform-state lookup, and runtime `coeffs()`.
- Wrong branch ordering around lower validation -> Mitigation: tests include a
  lossless input with invalid lower `txSet`/`UVMode` facts and require it to
  match explicit `DCT_DCT` behavior.
- Additional wrapper layering -> Mitigation: follow the existing staged
  ordinary-branch pattern and compare outputs against lower wrappers.
