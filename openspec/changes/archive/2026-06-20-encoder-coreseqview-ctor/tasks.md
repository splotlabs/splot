## 1. Minimal-intra CoreSeqView constructor

- [x] 1.1 Add `CoreSeqView::new_minimal_intra(single_picture_header_flag, max_frame_width, max_frame_height)` in `splot-core` (the inter view via its constructor + the six nested views all-disabled), preserving `#[non_exhaustive]`.
- [x] 1.2 Promote `base_seq()` to delegate to the constructor and remove the now-dead nested-view test helpers.
- [x] 1.3 Replace the remaining hand-rolled all-disabled `CoreSeqInterView` literal in the frame-header property tests with `CoreSeqInterView::new_minimal_intra()`.

## 2. Tests

- [x] 2.1 The promoted `base_seq()` keeps the frame-header round-trip suites green (the regression oracle).
- [x] 2.2 A direct test proves the parameterization: `single_picture_header_flag` reaches the seq/filter/CCSO views and the frame maxima are taken verbatim.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-WRITER-INPUT-SEQ-VIEW` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming a `FrameHeaderCore` constructor, tile-group OBU, frame, packet, `receive_packet` output, CLI success, or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused core tests, feature-status checks, and `cargo xtask ci`.
