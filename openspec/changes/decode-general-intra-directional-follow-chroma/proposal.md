## Why

The general intra decode reconstructs a single-block § 7.13.2.8 `D135_PRED`
(pAngle 135) luma block (`DECODE-GENERAL-INTRA-ANGLE`), but its chroma admission
accepts only `DC_PRED` and `SMOOTH_PRED`. The minimal-tool avmenc pairs a D135
luma block with the directional-follow chroma mode: when § 5.20.5.3
`read_intra_uv_mode` decodes `uv_mode == 0` over a directional luma,
`get_intra_uv_mode_set(0)` returns `YMode` itself and the spec sets
`AngleDeltaUV = AngleDeltaY`. For the supported luma D135 (`AngleDeltaY == 0`) the
chroma resolves to `UVMode == D135_PRED`, `AngleDeltaUV == 0` — a plain § 7.13.2.8
middle-angle chroma intra prediction. splot rejected this with the
`general_intra_non_dc_chroma_mode` diagnostic, which blocks any frame whose D135
block carries directional-follow chroma. Accepting and reconstructing it
(verified bit-exact against avmdec AND dav2d) unblocks future multi-block
directional fixtures.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-DIRECTIONAL-FOLLOW-CHROMA`.
- Add `SupportedChromaMode::D135Follow` and resolve `UVMode == D135_PRED` to it in
  `supported_chroma_mode`, ONLY for the directional-follow branch (`uv_mode == 0`
  and the luma is directional). A non-follow `D135_PRED` from the
  `Default_Mode_List_Uv` scan paired with a non-directional luma is left deferred
  (no oracle fixture reaches it).
- Reconstruct `D135Follow` chroma via the new
  `reconstruct_general_intra_chroma_directional_first_into`, which builds the
  § 7.13.2.8 middle-angle prediction over the § 7.13.2.1 no-neighbour fallback
  edges (the same `predict_directional_noneighbour` helper the luma D135 path
  uses) and adds the § 5.20.7.27 chroma residual.
- Gate `D135Follow` to the top-left (no-neighbour) 64x64 superblock (`n4w == 16`),
  where the edges reduce to the flat fallback so the `enableIdif == 0` bilinear
  middle-angle prediction equals the spec IDIF (shift `0`); a neighbour-having
  directional chroma block is rejected with a structured
  `decode/unsupported-feature` diagnostic.
- Explicitly keep CfL (`UV_CFL_PRED`), CCTX, and MHCCP chroma — which read
  separate cross-component syntax — out of scope and rejected.
- Add the project-owned `syn-dfchroma-intra-64x64-q80.ivf` fixture (a 64x64 D135
  luma block with 135-degree anti-diagonal chroma that avmenc codes as
  directional-follow D135 chroma) and prove it decodes bit-exactly to the avmdec
  AND dav2d oracle.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-directional-follow-chroma`: Crate-private general intra
  single-block directional-follow (`D135_PRED`) chroma decode over the § 7.13.2.1
  no-neighbour fallback edges.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra directional-follow chroma decode.

## Impact

- Affects `crates/splot-decode/src/tile_payload/cdf/block_context.rs` (the
  `SupportedChromaMode::D135Follow` variant and `supported_chroma_mode`
  resolution), `crates/splot-decode/src/runtime_minimal_recon.rs` (the new chroma
  directional reconstruction), and `crates/splot-decode/src/runtime_minimal.rs`
  (the admission diagnostic text and the `D135Follow` gate). No new public
  surface; the recon prediction helper is reused.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`, `tests/conformance/manifest.toml`, and
  generated status docs.
- No dependency graph, encoder, or validator changes. Neighbour-having directional
  chroma over a real non-flat edge (real § 7.13.2.8 chroma IDIF), other directional
  chroma angles and non-zero `AngleDeltaUV`, the non-follow `D135_PRED` scan
  pairing, CfL/CCTX/MHCCP chroma, SMOOTH_V/H / PAETH chroma, sub-superblock chroma,
  non-64x64 frames, inter prediction, and in-loop filters remain out of scope.
