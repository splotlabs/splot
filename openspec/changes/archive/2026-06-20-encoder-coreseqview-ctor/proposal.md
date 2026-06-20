## Why

Header-bridge brick 2 (after the `CoreSeqInterView` constructor). To drive
`write_tile_group_obu` / `write_frame_header_core`, the encoder needs a full
`CoreSeqView` — the sequence-derived view both the frame-header parser and the inverse
writer consume. It is `#[non_exhaustive]` with crate-private fields and is otherwise
only produced from a fully parsed `SequenceHeader` (`from_sequence`). This adds the
public minimal-intra constructor.

## What Changes

- Add `ENC-WRITER-INPUT-SEQ-VIEW` as an encoder writer-input-bridge feature (code in
  `splot-core`, driven by the encoder mission).
- Add `CoreSeqView::new_minimal_intra(max_frame_width, max_frame_height) ->
  Option<CoreSeqView>` in `splot-core` (the **non-single-picture** view; `None` for
  maxima outside `1..=2^16` since § 5.4.1 `frame_*_bits` is `f(4)`): the § 5.4.1 view a minimal intra
  frame needs — every unused sequence tool disabled (inter via
  `CoreSeqInterView::new_minimal_intra`, no segmentation/tiles/loop-filters/restoration/
  CCSO, no film grain), 8-bit YUV420, with the frame-size maxima as inputs and
  `frame_width_bits` / `frame_height_bits` derived from them (`ceil_log2(max).max(1)`).
  The single-picture variant (different § 5.4.1 inferences) is a separate later
  constructor.
- Promote the `base_seq()` test helper to delegate to the constructor, removing the
  now-dead nested-view test helpers, so the existing frame-header round-trip suite
  regresses it; add a direct parameterization test.
- Replace the remaining hand-rolled all-disabled `CoreSeqInterView` literal in the
  frame-header property tests with `CoreSeqInterView::new_minimal_intra()` (the
  follow-up to the brick-1 promotion).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the encoder writer-input minimal-intra
  `CoreSeqView` constructor.

## Impact

- Affected code: `crates/splot-core/src/headers/frame/info.rs` (the constructor, the
  `base_seq()` promotion + dead-helper removal, a parameterization test),
  `crates/splot-core/src/write/frame_header_core_proptests.rs` (the inter-view de-dup),
  `xtask/src/source_lines.rs` (the info.rs allowance reason; the file stays under the
  existing 5090 cap — net change is ~+9 after removing the dead helpers).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one new associated constructor on the existing public
  `splot-core` `CoreSeqView`. No dependency-graph change.
- Validator/CLI impact: none.
