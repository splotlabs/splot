## Why

The general intra decode path reconstructs DC, SMOOTH_V/SMOOTH_H, and D135
luma prediction bit-exactly. The § 7.13.2.13 SMOOTH family has one more mode:
plain `SMOOTH_PRED` (canonical § 9.2 mode 9), the full 2-D smooth that blends
BOTH the above row + top-right sentinel and the left column + bottom-left
sentinel (`Round2(predV2 + predH2, 1)`), distinct from SMOOTH_V (vertical only)
and SMOOTH_H (horizontal only). It is decoded as a non-escape § 5.20.5.3
`y_mode_index == 1` (`Reordered_Y_Mode[1] == SMOOTH_PRED`), which the mode
decode already resolves; the gap is purely the reconstruction admission.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-NONDC-LUMA-SMOOTH-PLAIN`.
- Map the reconstructed § 9.2 `SMOOTH_PRED` luma mode to a new
  `SupportedNonDcLumaMode::Smooth` (`IntraYMode::supported_nondc`).
- Map `SupportedNonDcLumaMode::Smooth` to the shared `splot-recon`
  `IntraSmoothMode::Smooth` in both the no-neighbour and neighbour smooth
  reconstruction entry points; the top-left no-neighbour reconstruction already
  flows through `predict_intra_smooth_rect_into` over the § 7.13.2.1 fallback
  edges plus the § 5.20.7.27 residual.
- Gate plain SMOOTH to the verified subset: the top-left (no-neighbour) 64x64
  superblock (`n4w == 16`, TX_64X64 -> § 5.20.8.2 `get_tx_set` `TX_SET_DCTONLY`)
  with DC chroma. Reject neighbour-having plain SMOOTH (which reads the real
  § 7.13.2.1 above-right `AboveRow[w]` / below-left `LeftCol[h]` sentinels) and
  sub-64x64 plain SMOOTH (mode-dependent non-DCT TxType) with a structured
  `decode/unsupported-feature` diagnostic before reconstruction.
- Add the project-owned `syn-smooth-intra-64x64-q124.ivf` fixture (plain SMOOTH
  luma, DC chroma) and prove it decodes bit-exactly to the avmdec/dav2d oracle,
  plus the negative `syn-smoothnondc-intra-64x64-q132.ivf` (plain SMOOTH luma +
  non-DC chroma) that the decoder still rejects.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-nondc-smooth-plain`: Crate-private single-block plain
  § 7.13.2.13 `SMOOTH_PRED` (2-D) luma intra prediction over the § 7.13.2.1
  no-neighbour fallback edges plus AC residual.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra single-block plain SMOOTH luma decode.

## Impact

- Affects `crates/splot-decode/src/tile_payload/cdf/block_context.rs`,
  `crates/splot-decode/src/runtime_minimal/general_intra.rs`, and
  `crates/splot-decode/src/runtime_minimal_recon.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `tests/conformance/manifest.toml`, and generated status docs.
- No public API, dependency graph, encoder, or validator changes. Neighbour-having
  plain SMOOTH, sub-64x64 plain SMOOTH, plain SMOOTH chroma, PAETH, non-64x64
  frames, inter prediction, in-loop filters, and live in-CI AVM/dav2d remain out
  of scope.
