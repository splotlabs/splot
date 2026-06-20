## Why

Header-bridge brick 3 (the keystone, after the `CoreSeqView` constructor). To emit a real
frame header the encoder needs a `FrameHeaderCore` — the § 5.18.2 model
`write_frame_header_core` / `write_tile_group_obu` serialize. `FrameHeaderCore` is
`#[non_exhaustive]` with crate-private fields and is otherwise only produced by the
parser. Rather than hand-build a field-by-field literal (which generalizes poorly), this
adds a **parse-backed** assembler: it serializes the canonical § 5.18.2 body for the
frozen 64x64, `base_q_idx == 255` single-picture `OBU_CLOSED_LOOP_KEY` intra frame and
parses it, so the result is conformant by construction (it inverts the same parser the
decoder runs). The decoder's frozen minimal tier requires `single_picture_header_flag`, so
this also adds the single-picture `CoreSeqView` variant the body is matched to.

## What Changes

- Add `ENC-FRAME-HEADER-CORE-ASSEMBLER` as an encoder writer-input-bridge feature (code in
  `splot-core`, driven by the encoder mission).
- Add `CoreSeqView::new_minimal_intra_single_picture(max_frame_width, max_frame_height) ->
  Option<CoreSeqView>`: the **single-picture** variant of `new_minimal_intra`, applying the
  exactly eight § 5.4.x inferences a single-picture sequence header forces and a non-single
  header signals differently — `single_picture_header_flag` (top-level + filter + CCSO),
  § 5.4.6 `OrderHintBits = 0` and `NumRefFrames = 2`, § 5.4.7
  `seq_force_screen_content_tools` / `seq_force_integer_mv = SELECT (= 2)`, § 5.4.8
  `(enable_avg_cdf, avg_cdf_type) = (true, 1)`, and § 5.4.1 `monotonic_output_order_flag =
  true`. Every other field stays the non-single view's disabled-tool value (a legal
  single-picture choice). Same `None` domain as `new_minimal_intra`.
- Add `build_minimal_intra_clk_core(seq: &CoreSeqView) -> Result<FrameHeaderCore,
  MinimalIntraCoreError>`: serialize the canonical § 5.18.2 body (`BitWriter`, one element
  per syntax field — `order_hint` omitted because `OrderHintBits == 0`, an explicit
  `allow_screen_content_tools` bit because SCC is `SELECT`, `uniform_tile_spacing_flag`
  with no increment bits because 64x64 is one superblock, `base_q_idx == 255`) and parse it
  against `seq` to an `IntraHeaderComplete` `FrameHeaderCore`.
- Add the `MinimalIntraCoreError` typed error (body-serialize / parse arms; both
  unreachable for the canonical input, present to honor the no-panic policy).
- Un-gate the existing `pub(crate)` re-export of `init_core_from_prefix` / `parse_core_body`
  (was `#[cfg(test)]`) so the non-test assembler can drive the lower-level core parser.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the encoder writer-input minimal-intra
  `FrameHeaderCore` parse-backed assembler (and its single-picture `CoreSeqView` input).

## Impact

- Affected code: `crates/splot-core/src/headers/frame/encoder_input.rs` (the single-picture
  constructor, the canonical-body serializer, the assembler, the typed error, the tests),
  `crates/splot-core/src/headers/frame/mod.rs` (un-gate + re-export).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one new associated constructor on the existing public `CoreSeqView`,
  one new public function `build_minimal_intra_clk_core`, one new public error
  `MinimalIntraCoreError`, all in `splot-core`. No dependency-graph change.
- Validator/CLI impact: none.
