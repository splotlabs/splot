## Context

The current LR transform-record residual path already preserves AV2 ordering by
reading `all_zero`, staging the nonzero EOB branch, and then deciding whether
post-EOB transform-tool syntax is supported. The former DCT-only gate resolves
active luma `transform_type()` syntax but rejects any mapped `PlaneTxType` other
than `DCT_DCT`.

The lower staged ordinary coefficient branch already accepts caller-resolved
`luma_tx_type`, derives `get_tx_class(PlaneTxType)`, computes §5.20.7.30 scan
order, and reads the ordinary non-FSC coefficient passes. The missing handoff is
therefore not a new coefficient parser; it is carrying the resolved luma
`PlaneTxType` out of the transform-tool syntax guard and into that existing
branch.

## Approach

1. Rename the DCT-only transform-tool metadata shape to describe the broader
   LR syntax handoff and add a `luma_tx_type` field initialized to `DCT_DCT`.
2. Change the active luma transform-type reader to return the resolved
   `PlaneTxType` instead of rejecting non-DCT values unconditionally.
3. Admit non-DCT luma `PlaneTxType` only for
   `ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff`; reconstruction-safe
   callers continue to reject before producing decoded samples.
4. Pass `metadata.luma_tx_type` into
   `CoeffOrdinaryBranchLosslessBaseConfig::luma_tx_type` so the staged ordinary
   branch derives scan order and coefficient contexts from the actual transform
   class.
5. Keep CCTX, active secondary inverse-transform semantics, reconstructed
   Quant safety, output, and reference refresh outside the feature boundary.

## Verification

- Add unit tests proving:
  - active luma non-DCT transform types are admitted for the LR handoff and
    retained as metadata;
  - reconstruction-safe policy still rejects the same non-DCT luma transform
    type;
  - the staged coefficient branch receives the actual luma `PlaneTxType`.
- Re-run the local ac0ej3 probe and update the ignored CLI test to the new
  structured frontier.
- Run focused decode tests, decoder-support/feature-status checks, OpenSpec
  validation, and `cargo xtask ci`.

## Non-Goals

- No inverse transform or residual-add support for non-DCT luma transforms.
- No decoded `CurrFrame`/`CdefFrame` population, loop-restoration filtering,
  raw/Y4M/hash success, reference refresh, or AVM/dav2d byte-equality claim.
- No broad `compute_tx_type()` runtime support outside the LR tx-skip
  syntax-only handoff.
