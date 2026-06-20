## ADDED Requirements

### Requirement: Encoder writer-input minimal-intra FrameHeaderCore parse-backed assembler

The encoder writer bridge SHALL provide a public `splot-core` parse-backed assembler for
the minimal-intra `FrameHeaderCore`, tracked by `ENC-FRAME-HEADER-CORE-ASSEMBLER`, so an
encoder can build the § 5.18.2 frame-header model that `write_frame_header_core` /
`write_tile_group_obu` require without a parsed `SequenceHeader` (the model is otherwise
`#[non_exhaustive]` and produced only by the parser). The assembler SHALL serialize the
canonical 64x64, `base_q_idx == 255` single-picture `OBU_CLOSED_LOOP_KEY` intra body and
parse it to an `IntraHeaderComplete` core, so the result is conformant by construction. It
SHALL be paired with a single-picture `CoreSeqView` constructor whose § 5.4.x inferences
make the body spec-real. It SHALL NOT produce a tile-group OBU, a frame, a packet, or
`Context::receive_packet` output.

#### Scenario: The single-picture constructor applies the spec inferences

- **WHEN** `CoreSeqView::new_minimal_intra_single_picture` is called with in-range maxima
- **THEN** the view SHALL differ from `new_minimal_intra` only by the eight § 5.4.x
  single-picture inferences (`single_picture_header_flag` top-level + filter + CCSO,
  § 5.4.6 `OrderHintBits = 0` and `NumRefFrames = 2`, § 5.4.7 `seq_force_screen_content_tools`
  / `seq_force_integer_mv = 2`, § 5.4.8 `(enable_avg_cdf, avg_cdf_type) = (true, 1)`,
  § 5.4.1 `monotonic_output_order_flag = true`)
- **AND** maxima outside `1..=2^16` SHALL yield `None`.

#### Scenario: The assembler produces a conformant intra core

- **WHEN** `build_minimal_intra_clk_core` is called with the single-picture view
- **THEN** it SHALL return a `FrameHeaderCore` with status `IntraHeaderComplete`, frame
  type Key, frame size 64x64, `order_hint_lsb == 0`, `refresh_frame_flags == 3`, and
  immediate (not implicit) output
- **AND** `write_frame_header_core` of that core SHALL re-emit a stream that reparses to an
  equal core.

#### Scenario: The bridge does not produce packets

- **WHEN** the assembler is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a tile-group OBU, a frame, a packet,
  or Baseline Encoder Profile v1 output from the assembler alone.
