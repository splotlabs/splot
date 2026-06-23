## Context

The general intra decode already reconstructs the § 7.13.2.13 SMOOTH_V and
SMOOTH_H luma modes for the top-left no-neighbour block: it builds the
§ 7.13.2.1 no-neighbour fallback edges and calls the shared `splot-recon`
`predict_intra_smooth_rect_into`, then adds the § 5.20.7.27 residual. The
`splot-recon` predictor already implements ALL three SMOOTH modes, including the
plain 2-D `IntraSmoothMode::Smooth` (`Round2(predV2 + predH2, 1)`). The decode
layer only admits SMOOTH_V/SMOOTH_H, so plain `SMOOTH_PRED` (canonical § 9.2
mode 9) is rejected as `general_intra_unsupported_luma_mode`.

The plain SMOOTH mode is decoded as a non-escape § 5.20.5.3
`y_mode_index == 1` (`Reordered_Y_Mode[1] == SMOOTH_PRED`), already handled by
`reconstruct_minimal_y_mode`. The only gap is the reconstruction admission.

## Decisions

- **Reuse the existing smooth path.** Plain SMOOTH is the same § 7.13.2.13
  process the SMOOTH_V/H path already runs; it differs only in the final blend
  (`Round2(predV2 + predH2, 1)` rather than `predV2` / `predH2`). Adding
  `SupportedNonDcLumaMode::Smooth` and the `IntraSmoothMode::Smooth` mapping in
  the two reconstruction entry points reuses `predict_intra_smooth_rect_into`
  with no new prediction code.
- **Top-left no-neighbour only.** At the top-left block both SMOOTH sentinels
  (the above-right `AboveRow[w]` and the below-left `LeftCol[h]`) are the
  § 7.13.2.1 no-neighbour fallback (8-bit `127` / `129`), so the 2-D blend is
  fully deterministic. Plain SMOOTH reads BOTH sentinels (unlike SMOOTH_V, which
  ignores the top-right value, and SMOOTH_H, which ignores the below-left
  value), so a neighbour-having plain SMOOTH block would need the real
  reconstructed above-right AND below-left sentinels over the § 5.20.2.3
  `BlockDecoded` state — not yet covered by an oracle fixture. The admission is
  therefore gated to the 64x64 top-left superblock and everything else is
  rejected with a structured diagnostic.
- **DC chroma only.** The supported chroma subset (DC, plus the directional
  follow) is unchanged. A plain-SMOOTH luma block paired with a non-DC chroma
  mode is rejected by the existing chroma gate; the committed negative fixture
  pins this boundary.

## Risks / Trade-offs

- The verified subset is narrow (one 64x64 top-left block). This is deliberate:
  admitting only what an oracle fixture proves bit-exact keeps the decoder from
  ever emitting a confident-but-wrong hash. Neighbour-having and sub-64x64 plain
  SMOOTH are deferred to dedicated future fixtures.

## Migration / Rollout

- No public API or data-format change. The change adds one enum variant and two
  match arms behind the existing crate-private general-intra entry points.
