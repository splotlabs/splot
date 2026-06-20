## 1. Minimal-intra inter view constructor

- [x] 1.1 Add `CoreSeqInterView::new_minimal_intra() -> CoreSeqInterView` in `splot-core` (all inter tools off, motion modes disabled), preserving `#[non_exhaustive]`.
- [x] 1.2 Promote the three `base_inter()` test helpers (info.rs, frame_header_core_tests.rs, tile_group_obu_tests.rs) to call the constructor.

## 2. Tests

- [x] 2.1 Prove the constructor's fields are all-disabled (direct field assertions; no `PartialEq`).
- [x] 2.2 The promoted `base_inter()` keeps the existing frame-header round-trip suites green (regression coverage at zero new-test cost).

## 3. Tracking and verification

- [x] 3.1 Add `ENC-WRITER-INPUT-INTER-VIEW` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming a CoreSeqView / FrameHeaderCore constructor, tile-group OBU, frame, packet, `receive_packet` output, CLI success, or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused core tests, feature-status checks, and `cargo xtask ci`.
