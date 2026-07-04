## Context

The active local decoder mission path reaches selectable Wiener NS LR transform-record
derivation and consumes mode info, MRL, transform-size records, and the
`all_zero` decision. It currently rejects every nonzero residual when sequence
transform tools such as FSC, CCTX, IDTX-intra, or intra-IST are enabled. That
guard is safe but coarse: AV2 §5.20.8.3 forces `TX_SET_DCTONLY` for some
transforms, and AV2 §5.20.8.2 can also read active luma transform-type symbols
whose mapped `TxType` is still `DCT_DCT`. The existing coefficient loop already
supports the ordinary DCT_DCT branch once caller facts are correct.

## Goals / Non-Goals

**Goals:**

- Admit nonzero residuals only when the actual per-plane transform path resolves
  to DCT_DCT and does not require unsupported non-DCT, CCTX, IST, or FSC syntax.
- Preserve fail-closed behavior for active MRL, FSC, CCTX, IST, and non-DCT
  transform-type branches.
- Improve diagnostics so the next local decoder mission frontier names the remaining active
  syntax instead of the old broad guard when DCT-only residuals are successfully
  consumed.

**Non-Goals:**

- No full `transform_type()` symbol parser for every luma/chroma/inter case:
  this slice covers only the active luma `intra_tx_type_set1`,
  `intra_tx_type_set2`, `is_long_side_dct`, and `intra_tx_type_long` cases
  needed to prove `DCT_DCT`; `inter_tx_type`, `sec_tx_type`, CCTX, and
  `most_probable_stx_set` remain out of scope.
- No CCTX coefficient transform, FSC coefficient branch admission, secondary
  transform reconstruction, decoded sample population, loop-restoration
  filtering/output, reference refresh, AVM/dav2d byte equality, or successful
  local decoder mission decode claim.

## Decisions

- Keep the admission decision at the residual caller boundary. The LR
  transform-record path already has the selectable `tx_size`, plane, chroma
  grouping, and sequence/frame facts needed to decide whether the residual is
  forced DCT-only before it asks the shared coefficient helper to read nonzero
  coefficients.
- Use spec-derived `get_tx_set` logic plus the generated luma transform-type
  mapping tables rather than a global sequence-tool guard. This follows AV2
  §5.20.8.3 and §5.20.8.2, avoids over-rejecting DCT_DCT luma transforms, and
  still rejects any mapped non-DCT transform before the coefficient loop can
  proceed with the wrong `PlaneTxType`.
- Leave the shared coefficient loop's `DCT_DCT` lower branch unchanged. The
  feature is an admission/handoff improvement, not a rewrite of coefficient
  decoding.

## Risks / Trade-offs

- [Risk] Accidentally admitting a transform that requires unimplemented syntax
  would desynchronize symbol reads. -> Mitigate by admitting only branches that
  resolve to `DCT_DCT`, keeping focused negative tests for mapped non-DCT types,
  and retaining `exit_symbol()` as the final byte-exactness guard.
- [Risk] The next local decoder mission block may require active CCTX/IST/FSC immediately. ->
  Mitigate with precise diagnostics and tracking so the following brick starts
  from the real next unsupported syntax.
