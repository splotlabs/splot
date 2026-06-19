## Context

The ordinary coefficient branch is being staged from explicit caller facts
toward runtime AV2 section 5.20.7.27 `coeffs()` integration. The current
transform-type handoff covers non-lossless intra chroma `UVMode` mapping,
including directional `wide_angle_mapping`, but still rejects luma input with
`UnsupportedModeToTxfmSubset { reason: "luma" }`.

AV2 section 5.20.7.29 defines the non-lossless luma path after `get_tx_set` as
`return TxTypes[blockY][blockX]`. The runtime frame/block state that owns
`TxTypes` is not wired into this staged branch yet, so the narrow handoff must
carry a caller-resolved luma transform type.

## Goals / Non-Goals

**Goals:**

- Add a loaded-but-unwired decode-local handoff for non-lossless luma
  `TxTypes`, tracked by `DECODE-COEFF-ORDINARY-BRANCH-LUMA-TXTYPES-HANDOFF`.
- Return the caller-resolved luma transform type before chroma-only
  `enable_chroma_dctonly`, inter-chroma, or `UVMode` logic.
- Validate the caller-resolved luma transform-type domain before CDF, symbol, or
  coefficient-context mutation.
- Preserve existing all-zero, chroma non-directional, chroma directional,
  chroma-DCT-only, transform-set fallback, and chroma unsupported-subset
  behavior.

**Non-Goals:**

- Runtime `coeffs()` wiring.
- Deriving `TxTypes` from frame state.
- Chroma inter `TxTypes` lookup.
- FSC/IDTX lossless branches.
- Parsing block syntax facts.
- Dequantization, inverse transform, residual add, output/reference refresh, or
  AVM/dav2d byte-match proof.

## Decisions

- Reuse the existing transform-type handoff rather than adding a new wrapper.
  The handoff already owns the staged § 5.20.7.29 transform-type split and feeds
  the downstream transform-size/scan branch.
- Add `luma_tx_type` to `CoeffOrdinaryBranchModeToTxfmBaseConfig`,
  `CoeffOrdinaryBranchTxSetBaseConfig`, and
  `CoeffOrdinaryBranchLosslessBaseConfig`. Existing chroma tests default the
  value to `DCT_DCT`; luma-focused tests set non-DCT values to prove the branch
  is not a DCT-only shortcut.
- Validate `luma_tx_type` against the AV2 `TX_TYPES` domain before delegation.
  The lower explicit `PlaneTxType` handoff remains deliberately broad for staged
  tests, but the `TxTypes` handoff models a finite syntax/state table and should
  reject impossible caller facts atomically.

## Risks / Trade-offs

- [Risk] Adding another caller-resolved field touches multiple staged config
  builders. -> Mitigation: default it to `DCT_DCT` in existing helpers and add
  focused luma tests.
- [Risk] The lower explicit path accepts transform-type values outside the AV2
  domain for transform-class fallback testing. -> Mitigation: keep that behavior
  unchanged and validate only the new `TxTypes` handoff boundary.
- [Risk] This still does not broaden runtime decode output. -> Mitigation:
  track it as partial in the implementation and decoder support matrices and
  keep roadmap wording explicit.
