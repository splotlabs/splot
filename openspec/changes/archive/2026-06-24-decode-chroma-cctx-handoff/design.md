## Context

The current `local-decoder-mission.ivf` live probe reaches the active Wiener NS LR transform-record path and decodes several active luma transform-type branches before failing on chroma residual syntax. Temporary instrumentation showed the first chroma stop is U-plane `tx_size == 6`, `eob == 1`, `tx_set == TX_SET_INTRA_1`; a temporary eob-1 chroma admission then reaches U-plane `tx_size == 5`, `eob == 13` with active `enable_cctx`.

AV2 §5.20.7.27 reads `cctx_type` for plane 1 when `(is_inter || eob != 1) && is_cctx_allowed()`. AV2 §8.3.2 selects `TileCctxTypeCdf` for that symbol. The existing tile CDF subset has the generated default in `splot-core` but does not expose it through `splot-decode` yet.

## Goals / Non-Goals

**Goals:**
- Wire `TileCctxTypeCdf` through the tile CDF subset, selector, symbol-read helper, tests, and lifecycle state.
- Add a policy-scoped residual syntax mode for the Wiener NS LR tx-skip handoff that can read `cctx_type` and retain the decoded value as syntax metadata.
- Let the same LR-only policy consume intra chroma coefficient syntax when `get_tx_set` does not force DCT_DCT after recording any required CCTX type, relying on the existing ordinary coefficient handoff to derive the real chroma transform type.
- Advance the local `local-decoder-mission.ivf` probe to the next structured unsupported frontier without producing output.

**Non-Goals:**
- Do not implement CCTX transform reconstruction or cross-chroma-component residual mixing.
- Do not claim broad non-DCT chroma reconstruction or output support.
- Do not make the general reconstruction-safe residual policy admit these branches.
- Do not claim bit-exact `local-decoder-mission.ivf` decode against AVM/dav2d.

## Decisions

1. **Add CCTX as a normal tile CDF row.** `TileCctxTypeCdf` belongs in the same `TileCdfSelector` and lifecycle machinery as the transform-type and coefficient rows. This keeps symbol reads traceable to AV2 §8.3.2 and lets update-mode tests cover row mutation behavior.

2. **Keep admission policy-scoped.** The existing `AdmitDctOnly` residual policy already distinguishes reconstruction-safe callers from the LR tx-skip record handoff through `ActiveIntraIstResidualPolicy`. Extend that policy surface rather than changing the broad `Allow` path or removing fail-closed checks. Reconstruction-safe callers must still reject active CCTX/non-DCT chroma residuals.

3. **Record CCTX as syntax metadata only.** Reading `cctx_type` is required for bitstream sync, but its value changes dequant/reconstruction semantics only after coefficient parsing. The LR tx-skip handoff may consume and store the syntax value, but no decoded samples or output may use it until CCTX reconstruction is implemented and oracle-verified.

4. **Use existing coefficient transform-type derivation for chroma syntax.** For intra chroma residuals, the ordinary coefficient handoff already derives `PlaneTxType` from `UVMode`, `txSet`, and `enable_chroma_dctonly`; the LR handoff only needs the parsed eob/tx-skip facts. This avoids pretending the chroma block is DCT_DCT while still keeping decoded Quant out of reconstruction.

## Risks / Trade-offs

- **Risk:** Over-admitting CCTX syntax could make a wrong hash possible later.
  **Mitigation:** Gate this path to LR tx-skip record handoff and keep all CCTX reconstruction/output unsupported; no output is produced.

- **Risk:** Chroma transform-set admission could mask a future non-DCT reconstruction gap.
  **Mitigation:** The policy name and matrix row will state syntax-only LR handoff; reconstruction-safe callers keep rejecting.

- **Risk:** CDF lifecycle omissions could desync later symbol reads.
  **Mitigation:** Add selector, default-copy, symbol-read, and update-mode tests for `TileCctxTypeCdf`.
