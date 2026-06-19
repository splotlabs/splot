## Context

`ENC-INTRA-MODE-SYMBOL-EMISSION` emits the luma intra-mode selectors
(`y_mode_set`, `y_mode_index`). The next selector in a coded intra block is the
chroma `uv_mode` (AV2 §5.20.5.6), which selects the prediction mode shared by both
chroma planes. This change emits the minimal DC chroma `uv_mode` token and proves
it through the in-tree AV2 §8.2 symbol coder, reusing the `intra_mode_emission`
module's token / roundtrip machinery.

In the AV2 §5.20.5.3 mode-info order, `intra_frame_mode_info()` calls
`read_intra_uv_mode()` right after `read_intra_y_mode()` and before `residual()`,
so `uv_mode` precedes all coefficient symbols. This change emits `uv_mode` as a
standalone token for ordered composition (`y_mode_set`, `y_mode_index`, `uv_mode`,
then residual/coefficient syntax). `read_intra_uv_mode()` (§5.20.5.6) also reads
`use_dpcm_uv` (when `Lossless`) and `is_cfl` (when `cflAllowed ||
is_mhccp_allowed()`) before `uv_mode`; this change is valid only for the minimal
tier where neither is read — a non-lossless block with CfL disabled
(`enable_cfl_intra == 0`) and MHCCP unavailable.

Normative AV2 v1.0.0 sections:

- §5.20.5.6 Read intra UV mode syntax
  (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-6`) — the `uv_mode`
  syntax element and the `Default_Mode_List_Uv` index→mode mapping (index 0 =
  DC_PRED for a non-directional luma mode).
- §8.3.2 CDF selection: `TileUVModeCflNotAllowedCdf[ctx]`, `ctx =
  is_directional(YMode)`, which is 0 for the DC_PRED luma block.

## Goals / Non-Goals

**Goals:**

- Add `emit_minimal_dc_chroma_uv_mode` for `ENC-UV-MODE-SYMBOL-EMISSION`, emitting
  the ordered `uv_mode=0` (DC chroma) token record at the non-directional context
  0 with its scoped §8.3.2 CDF selector.
- Prove the token value writes through the in-tree AV2 §8.2 symbol encoder with
  the scoped default CDF row and decodes back to the same value.
- Preserve the no-packet invariant in `Context`.

**Non-Goals:**

- No chroma prediction modes other than DC (the simplest legal choice; the
  decoder fixture's H_PRED is one valid alternative the encoder need not match).
- No CfL / CCTX, directional-luma `uv_mode` contexts, coefficient/all-zero
  symbols, partition syntax, tile CDF lifecycle, tile-body emission, packet
  output, CLI success, or Baseline Encoder Profile v1 claim.
- No dependency graph change and no AVM/dav2d evidence; this helper emits no
  stream.

## Decisions

1. **Reuse the `intra_mode_emission` module.** `uv_mode` is an intra-mode
   selector, so it extends the existing token / CDF-rows / §8.2 roundtrip
   machinery rather than duplicating it. The module advances a second focused
   Feature ID (`ENC-UV-MODE-SYMBOL-EMISSION`).

2. **Emit DC chroma (`uv_mode=0`).** `Default_Mode_List_Uv[0] = DC_PRED` for a
   non-directional luma mode, the simplest legal chroma mode and the one whose
   reconstruction (`splot-recon` DC chroma prediction) the closed loop can use.

3. **Tile-origin / non-directional context only.** The CDF context is
   `is_directional(YMode)`; for the DC_PRED minimal tier that is 0. The CDF-rows
   accessor holds only that row and rejects other contexts, matching the
   tile-origin restriction already applied to `y_mode_index`.

4. **Reuse the typed error model.** The existing `IntraModeEmission*` errors are
   syntax-agnostic (keyed by a `&'static str` syntax name), so `uv_mode` needs no
   new error variants.

## Flight Manifest

- Change ID: `encoder-uv-mode-emission`
- Feature IDs: `ENC-UV-MODE-SYMBOL-EMISSION`
- Base commit: `9a48651e` (`feat(encode): add minimal intra-mode symbol emission (#306)`)
- Depends on merged changes: `encoder-intra-mode-symbol-emission`,
  `encoder-coefficient-tokenization-minimal`, `range-encoder-complete`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/intra_mode_emission.rs`
  - `crates/splot-encode/src/lib.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-uv-mode-emission/**`
  - `openspec/changes/archive/2026-06-19-encoder-uv-mode-emission/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - `crates/splot-core/**`, `crates/splot-decode/**`, `crates/splot-recon/**`,
    `crates/splot-validate/**`, `crates/splot-cli/**`
  - `crates/splot-encode/src/error.rs` (no new error variants needed)
  - workspace manifests and `Cargo.lock`
  - AV2 spec mirror files under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-UV-MODE-SYMBOL-EMISSION`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `9a48651e`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  opens/lands first, the only expected overlap is the generated/tracking docs;
  sync `main`, regenerate, and re-gate before merge.
- Semantic overlap with each sibling PR: none; private encoder mode emission.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] A second matrix row shares the `intra_mode_emission` module. ->
  Mitigation: the `module` field is informational; both rows are focused, and the
  proof tests are distinct (`uv_mode` tests vs `y_mode` tests).
- [Risk] Emitting only DC chroma is narrow. -> Mitigation: matches the minimal
  closed-loop tier; broader chroma modes / CfL arrive with their own Feature IDs
  and exact reconstruction support.
