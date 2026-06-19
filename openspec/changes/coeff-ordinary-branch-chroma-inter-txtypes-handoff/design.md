## Context

The ordinary coefficient branch is being staged from explicit caller facts
toward runtime AV2 section 5.20.7.27 `coeffs()` integration. The current
transform-type handoff covers non-lossless luma `TxTypes`, chroma
`enable_chroma_dctonly`, and chroma intra `UVMode` mapping, including
directional `wide_angle_mapping`, but still rejects chroma inter input after the
chroma-DCT-only shortcut.

AV2 section 5.20.7.29 defines the non-lossless chroma-inter path as deriving
`x4 = Max(MiCol, blockX << SubsamplingX)`, `y4 = Max(MiRow, blockY <<
SubsamplingY)`, loading `TxTypes[y4][x4]`, checking membership with
`is_tx_type_in_set(txSet, txType)`, falling back to `DCT_DCT` when absent, and
otherwise returning `txType`. Runtime frame/block state for `TxTypes`, `MiRow`,
`MiCol`, and chroma subsampling is not wired into this staged branch yet, so the
narrow handoff must carry caller-resolved chroma-inter facts.

## Goals / Non-Goals

**Goals:**

- Add a loaded-but-unwired decode-local handoff for non-lossless chroma-inter
  `TxTypes`, tracked by
  `DECODE-COEFF-ORDINARY-BRANCH-CHROMA-INTER-TXTYPES-HANDOFF`.
- Return `DCT_DCT` when caller-resolved chroma-inter `TxTypes` is outside
  `Tx_Type_In_Set_Inter[txSet]`; otherwise return the caller value.
- Validate caller-resolved chroma-inter transform-type and inter transform-set
  domains before CDF, symbol, or coefficient-context mutation.
- Preserve existing all-zero, luma, chroma-DCT-only, chroma intra
  non-directional, chroma intra directional, and lossless unsupported-subset
  behavior.

**Non-Goals:**

- Runtime `coeffs()` wiring.
- Deriving `TxTypes`, `MiRow`, `MiCol`, `SubsamplingX`, or `SubsamplingY` from
  frame state.
- FSC/IDTX lossless branches.
- Parsing block syntax facts.
- Dequantization, inverse transform, residual add, output/reference refresh, or
  AVM/dav2d byte-match proof.

## Decisions

- Reuse the existing transform-type handoff rather than adding a new wrapper.
  The handoff already owns the staged AV2 section 5.20.7.29 split and delegates
  to the downstream transform-size/scan branch.
- Add `chroma_inter_tx_type` to `CoeffOrdinaryBranchModeToTxfmBaseConfig`,
  `CoeffOrdinaryBranchTxSetBaseConfig`, and
  `CoeffOrdinaryBranchLosslessBaseConfig`. Existing intra/luma tests default the
  value to `DCT_DCT`; chroma-inter tests set non-DCT values to prove both
  membership and fallback paths.
- Add a cited inline `Tx_Type_In_Set_Inter` table beside the existing inline
  intra table. The table is specified directly in AV2 section 5.20.7.29 rather
  than generated in a shared table module today, and this change keeps the
  staging local to `splot-decode`.
- Validate `chroma_inter_tx_type` against the AV2 `TX_TYPES` domain and validate
  `txSet` against `Tx_Type_In_Set_Inter` before delegation. Invalid caller facts
  return typed ordinary branch errors without reading symbols or mutating state.

## Risks / Trade-offs

- [Risk] Adding another caller-resolved field touches multiple staged config
  builders. -> Mitigation: default it to `DCT_DCT` in existing helpers and add
  focused chroma-inter tests.
- [Risk] This handoff still does not compute `x4`/`y4` or read runtime
  `TxTypes`. -> Mitigation: track the feature as partial and keep roadmap,
  support-matrix, and memory wording explicit.
- [Risk] The inline inter membership table duplicates spec text in code. ->
  Mitigation: keep it short, cite AV2 section 5.20.7.29, and test representative
  allowed/fallback rows through behavior rather than broad table snapshots.
