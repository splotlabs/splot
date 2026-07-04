## Why

Decoding any real sequence (local decoder mission is ~12961 frames, each referencing prior
decoded frames) needs the decoder to retain MORE than one decoded frame and let a
later frame select among them. So far the multi-frame runtime caps at a key plus
ONE inter frame, retains only the key in reference slot 0, and the AV2 § 7.7
implicit reference-map derivation is gated to the at-most-one-valid-slot case
(`derive_implicit_ref_map` stops with `UnmodeledDerivation` for two valid slots
because the § 7.7 ranking scores `RefBaseQIdx`, which the reference-state view did
not model). The § 5.20.7.12 `read_single_ref` entropy element
(`DECODE-INTER-SINGLE-REF-SYMBOL`) is loaded but unwired for the same reason: it is
only read when `NumTotalRefs >= 2`.

This change wires the multi-reference runtime: it retains a second decoded (inter)
frame via the § 7.20 / § 7.23 reference frame update, runs the real § 7.7 ranking
over two valid slots, and reads `single_ref` to select the reference. It is proven
bit-exact by a committed three-frame fixture whose third frame selects the RETAINED
INTER frame (slot 1), distinct from the key (slot 0), so the selection is
falsifiable (asymmetric output).

## What Changes

- Add Feature ID `DECODE-INTER-MULTIREF-RUNTIME` to the implementation matrix and a
  partial decoder-support row `inter-multiref-runtime`.
- splot-core: add `RefBaseQIdx` to `FrameReferenceStateView` (the new
  `from_slots_with_base_q_idx` constructor) and feed it into the § 7.7
  `derive_implicit_ref_map` scoring, lifting the `valid_count > 1` →
  `UnmodeledDerivation` gate WHEN `RefBaseQIdx` is modeled (every other § 7.7
  scoring input is deterministic for the single-spatial-layer minimal frame). The
  historical `from_slots` view (no `RefBaseQIdx`) STAYS an `UnmodeledDerivation`
  stop.
- splot-decode: add a `RuntimeReferenceBuffer` that applies the § 7.23 reference
  frame update per frame's `refresh_frame_flags` (KEY/SWITCH: `RefValid[i] =
  first`; inter: `RefValid[i] = 1`), stores per-slot `RefValid` / `RefOrderHint` /
  dims / `RefBaseQIdx` + the decoded frame index, and builds the borrowed
  `ReferenceFrameStore` for the next inter frame. Extend the multi-frame driver to
  decode a key plus up to two inter frames (3 + 2·(N − 1) OBUs).
- splot-decode: wire § 5.20.7.12 `read_single_ref` into the inter block decode
  (between `read_skip` and `single_mode`) when § 7.7 yields `NumTotalRefs == 2`,
  deriving the § 8.3.2 `single_ref` context from the neighbour `count_refs`
  (cross-checked vs AVM `av2_get_ref_pred_context`), and resolve the per-block
  reference slot via `ref_frame_idx[RefFrame[0]]`.
- Commit `syn-3frame-multiref-64x64.ivf` (3-oracle-verified) and prove the whole
  stream decodes bit-exact, including a reference-retention test proving frame 2
  reads frame 1's samples (not the key's).

## Capabilities

### New Capabilities
- `decode-inter-multiref-runtime`: Retains a second decoded inter frame in the
  § 7.23 reference buffer, derives the § 7.7 implicit reference map over two valid
  slots (modeling `RefBaseQIdx`), and reads § 5.20.7.12 `single_ref` to
  motion-compensate the selected reference, proven bit-exact vs avmdec and dav2d.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row and flip the
  `single_ref` entropy-element row to WIRED.

## Impact

- splot-core: `FrameReferenceStateView` gains a non-breaking `ref_base_q_idx`
  field + `from_slots_with_base_q_idx` constructor; `derive_implicit_ref_map`
  feeds it. No public API removed; the `#[non_exhaustive]` view stays
  backward-compatible (`from_slots` callers unchanged).
- splot-decode: new `runtime_minimal/reference_buffer.rs`, multi-frame driver
  changes, inter block `single_ref` wiring, and the § 8.3.2 `single_ref` context.
- No dependency-graph, encoder, or validator changes. Diagnostics: only the
  existing `decode/unsupported-feature` (new reasons for the rejected sub-cases).
- VERIFIED-SUBSET DISCIPLINE — rejected before any output: `NumTotalRefs > 2`,
  compound / `reference_select`, a 4th frame, a neighbour-having `single_ref`
  block (the § 8.3.2 ctx is gated to the no-neighbour ctx 1 the fixture proves),
  more than two valid reference slots, and a frame that would inherit an ADAPTED
  inter frame's CDFs (the decoder does not model § 7.23 cross-frame CDF
  save/load; the fixture uses `--cdf-update-mode=0` so every frame's
  `disable_cdf_update == 1`).
- Out of scope (named follow-on): § 7.23 cross-frame CDF save/load, `NumTotalRefs
  > 2` / multi-decision `single_ref`, a neighbour-having `single_ref` context,
  compound references (`read_compound_ref`, § 5.20.7.11), temporal MV
  (ref-frame-mvs), and the deferred § 7.12.2 ref-MV-bank / DRL-reorder / warp
  candidates.
