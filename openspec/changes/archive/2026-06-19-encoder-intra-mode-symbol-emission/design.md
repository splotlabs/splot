## Context

`ENC-COEFFICIENT-TOKENIZATION-MINIMAL` already emits the coefficient-loop token
records (`all_zero`, `eob_pt_16`, `coeff_base_eob`, `dc_sign`) for a 4x4 luma
block, and `ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL` closes the decoder-visible
reconstruction loop. The remaining missing piece before a coded tile body is the
block-level *mode* syntax that precedes coefficients. The first such symbols are
the luma intra mode selectors `y_mode_set` and `y_mode_index` (AV2 §5.20.5.5),
which together select the luma prediction mode.

This change is still not packet output. It produces ordered §8.3.2 token records
for the minimal DC_PRED luma block at the tile origin and proves them through the
in-tree AV2 §8.2 `splot-core` symbol encoder/decoder, exactly mirroring the
`coefficient_tokenization` module's proven pattern. The chroma mode (`uv_mode`,
§5.20.5.6) and the all-zero/coefficient symbols are out of scope here.

Normative AV2 v1.0.0 sections:

- §5.20.5.5 Read intra Y mode syntax
  (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-5`) — the `y_mode_set` /
  `y_mode_index` syntax elements.
- §8.3.2 CDF selection / contexts (the `TileYModeSetCdf` and
  `TileYModeIndexCdf[ctx]` rows; `y_mode_set` has no context, `y_mode_index` uses
  the joint-neighbour context, which is 0 at the tile origin where both
  neighbours are out of frame).
- §7.13.2.10 DC intra prediction (the prediction `y_mode_set=0`, `y_mode_index=0`
  resolves to — DC_PRED — already used by the closed loop).

## Goals / Non-Goals

**Goals:**

- Add a private `intra_mode_emission` module for `ENC-INTRA-MODE-SYMBOL-EMISSION`.
- Emit the ordered `y_mode_set=0` and `y_mode_index=0` token records for the
  minimal DC_PRED luma block at the tile-origin neutral context, with their
  scoped §8.3.2 CDF selectors.
- Prove the token values write through the in-tree AV2 §8.2 symbol encoder with
  the scoped default CDF rows and decode back to the same values.
- Preserve the no-packet invariant in `Context`.

**Non-Goals:**

- No chroma `uv_mode` (§5.20.5.6), all-zero/coefficient symbols, partition
  symbols, tile CDF lifecycle, tile-body emission, packet output, CLI success, or
  Baseline Encoder Profile v1 claim.
- No intra modes other than DC_PRED, and no neighbour-derived `y_mode_index`
  contexts beyond the tile-origin neutral context.
- No dependency graph change and no AVM/dav2d evidence; this helper emits no
  stream.

## Decisions

1. **Keep emission private and loaded-but-unwired.** The module is loaded from
   `splot-encode` and exposes only crate-private types and helpers, matching the
   coefficient-tokenization, residual, transform, quantization, and closed-loop
   foundations. It does not change `Context::receive_packet`.

2. **Represent token facts, not bytes.** The primary output is an ordered list of
   scoped token records; the §8.2 roundtrip is test/proof evidence, not a tile
   payload buffer or packet boundary.

3. **DC_PRED at the tile origin only.** The minimal subset emits `y_mode_set=0`
   and `y_mode_index=0`, which resolve to DC_PRED, using the tile-origin
   `y_mode_index` context 0 (both neighbours out of frame). Other modes and
   neighbour contexts are rejected/omitted and added later with their own Feature
   IDs.

4. **Use scoped default CDF rows for roundtrip proof only.** The roundtrip uses
   `splot-core`'s generated `DEFAULT_Y_MODE_SET_CDF` and `DEFAULT_Y_MODE_INDEX_CDF`
   default rows to encode and decode the token values, keeping tile CDF lifecycle
   and broad §8.3.2 selector completeness out of scope.

## Flight Manifest

- Change ID: `encoder-intra-mode-symbol-emission`
- Feature IDs: `ENC-INTRA-MODE-SYMBOL-EMISSION`
- Base commit: `f3e8575d` (`feat(encode): add minimal closed-loop reconstruction (#303)`)
- Depends on merged changes: `encoder-coefficient-tokenization-minimal`,
  `range-encoder-complete`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/intra_mode_emission.rs`
  - `crates/splot-encode/src/error.rs`
  - `crates/splot-encode/src/lib.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-intra-mode-symbol-emission/**`
  - `openspec/changes/archive/2026-06-19-encoder-intra-mode-symbol-emission/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - `crates/splot-core/**`
  - `crates/splot-decode/**`
  - `crates/splot-recon/**`
  - `crates/splot-validate/**`
  - `crates/splot-cli/**`
  - workspace manifests and `Cargo.lock`
  - AV2 spec mirror files under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-INTRA-MODE-SYMBOL-EMISSION`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `f3e8575d`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  opens and lands first, the only expected overlap is the generated/tracking docs
  (`docs/IMPLEMENTATION-MATRIX.toml`, `docs/FEATURE-STATUS.md`,
  `docs/SPEC-COVERAGE.md`); rebase and regenerate before merge.
- Semantic overlap with each sibling PR: none; this change is private encoder
  mode emission and does not depend on decoder code.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] The tiny two-symbol mode surface can be mistaken for a usable tile body.
  -> Mitigation: keep it private, preserve `receive_packet` behavior, and make
  the matrix/docs explicitly exclude chroma, coefficients, and packet output.
- [Risk] Default-CDF roundtrips are narrower than a real tile CDF lifecycle. ->
  Mitigation: call them proof rows for token/symbol compatibility only and leave
  tile CDF state to later Feature IDs.
- [Risk] Emitting only DC_PRED is narrow. -> Mitigation: this matches the existing
  minimal closed-loop tier; broader intra modes arrive with their own Feature IDs
  and reconstruction support.
