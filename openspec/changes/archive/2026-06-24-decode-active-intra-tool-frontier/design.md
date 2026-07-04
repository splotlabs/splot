## Context

The current local decoder mission key frame reaches `TX_MODE_SELECT` Wiener NS LR transform-record derivation, but the selectable path still rejects broad sequence flags before tile syntax. The local stream has `enable_mrls`, `enable_intra_edge_filter`, `enable_ibp`, `enable_fsc`, `enable_idtx_intra`, `enable_intra_ist`, and `enable_cctx` set, while the observed safe subset can be decided only after reading block mode and residual syntax.

## Goals / Non-Goals

**Goals:**
- Move the coarse sequence-level selectable transform-record tool gate into active-use checks.
- Add tile CDF support for AV2 §9.3 MRL rows and consume AV2 §5.20.5.5 `mrl_index` / `mrl_sec_index` in spec order.
- Admit `mrl_index == 0` directional blocks and reject nonzero MRL as the next precise unsupported frontier.
- Reject nonzero residual branches before unsupported transform-type/CCTX/IST syntax would be skipped.
- Keep local decoder mission decode fail-closed and update support tracking.

**Non-Goals:**
- Implement nonzero MRL prediction, intra-edge filtering, IBP prediction, transform-type symbol parsing, CCTX, IST, decoded sample output, loop-restoration filtering, reference refresh, or AVM/dav2d byte equality.
- Add dependencies or change public decoder APIs.

## Decisions

1. Expose MRL CDF rows through `TileCdfSelector`.

   The generated defaults already live in `splot-core`. The tile CDF subset will add `MrlIndex` and `MrlSecIndex` selectors, include them in tile/default rows, mutable selection, copy/average, and frame-end scaling. This keeps arithmetic adaptation transactional with the rest of the tile rows.

2. Carry MRL enablement through the existing general intra mode config.

   `GeneralIntraChromaToolConfig` is already passed to luma and chroma mode parsing for sequence-driven mode-info syntax. Extending it with `enable_mrls` avoids a parallel config type and keeps §5.20.5.5 handling local to mode parsing.

3. Consume MRL syntax before rejecting active nonzero MRL.

   The decoder will read `mrl_index` when `enable_mrls && is_directional_mode(YMode)`. If it is nonzero, it will read `mrl_sec_index` when required and then return a structured unsupported diagnostic. This preserves arithmetic synchronization for the inactive zero case and fails closed for active prediction semantics.

4. Treat transform-tool sequence flags as active-use gates, not sequence gates.

   For all-zero transform blocks, §5.20.7.27 does not reach `transform_type()` or CCTX. For nonzero blocks under the enabled local decoder mission transform-tool set, the decoder will reject before continuing into coefficient parsing because the current helper does not consume transform-type/CCTX/IST syntax.

## Risks / Trade-offs

- [Risk] The local stream may hit active nonzero MRL immediately. → Mitigation: the result is still progress because the diagnostic identifies the real active syntax frontier instead of a broad sequence gate.
- [Risk] Relaxing sequence gates could accidentally allow output with prediction-affecting tools. → Mitigation: this path is only for transform-record/LR handoff and remains before decoded sample output; support rows and diagnostics explicitly exclude reconstruction/output.
- [Risk] Adding CDF rows without lifecycle coverage would drift later frames. → Mitigation: include default, mutable, copy/average, and frame-end scaling paths plus focused tests.
