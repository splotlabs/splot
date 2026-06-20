## Why

Header-bridge brick 1 (toward the encoder driving `write_tile_group_obu`). To build a
`CoreSeqView` for the writer, the encoder needs its nested `CoreSeqInterView` — the one
nested sequence sub-view that is `#[non_exhaustive]` with crate-private fields and has
no public constructor (the other six sub-views are already publicly constructible). A
design pass (mappers + adversarial verify) confirmed this is the only blocked nested
view and that an intra frame never reads it (the § 5.18.2 control region skips the inter
tail), so an all-disabled view is the inert state the writer needs.

## What Changes

- Add `ENC-WRITER-INPUT-INTER-VIEW` as an encoder writer-input-bridge feature (code in
  `splot-core`, driven by the encoder mission).
- Add `CoreSeqInterView::new_minimal_intra() -> CoreSeqInterView` in `splot-core`: the
  all-disabled § 5.4.6 inter view (every inter tool off, every motion mode disabled),
  the value the existing `base_inter()` test helpers build by hand.
- Promote the three `base_inter()` writer/parser test helpers to call the new
  constructor, so the existing frame-header round-trip suites regress it.
- Prove the constructor's fields are all-disabled (direct field assertions; the type
  has no `PartialEq`).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the encoder writer-input minimal-intra inter
  view constructor.

## Impact

- Affected code: `crates/splot-core/src/headers/frame/info.rs` (the constructor + a
  field test + the promoted `base_inter`), `crates/splot-core/src/write/frame_header_core_tests.rs`
  and `crates/splot-core/src/write/tile_group_obu_tests.rs` (promoted `base_inter`).
  No new types, no signature changes; `#[non_exhaustive]` preserved.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one new associated constructor on the existing public
  `splot-core` `CoreSeqInterView`. No dependency-graph change.
- Validator/CLI impact: none.
