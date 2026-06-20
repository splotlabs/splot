## ADDED Requirements

### Requirement: Encoder writer-input minimal-intra CoreSeqView constructor

The encoder writer bridge SHALL provide a public `splot-core` constructor for the
minimal-intra `CoreSeqView`, tracked by `ENC-WRITER-INPUT-SEQ-VIEW`, so an encoder can
build the § 5.4.1 sequence-derived input that `write_tile_group_obu` /
`write_frame_header_core` require without a parsed `SequenceHeader` (the model is
otherwise `#[non_exhaustive]` and built only from `from_sequence`). The constructor
SHALL return the view with every unused sequence tool disabled (the inter view via its
own constructor, no segmentation/tiles/loop-filters/restoration/CCSO, no film grain),
8-bit YUV420, parameterized by `single_picture_header_flag` and the § 5.4.1 frame-size
maxima; `single_picture_header_flag` SHALL propagate to the filter view and the CCSO
view. The round-trip-proven `base_seq()` test helper SHALL delegate to it so the
frame-header round-trip suite regresses it. It SHALL NOT provide a `FrameHeaderCore`
constructor, a tile-group OBU, a frame, a packet, or `Context::receive_packet` output.

#### Scenario: The constructor propagates the parameters

- **WHEN** the constructor is called with `single_picture_header_flag = true` and frame
  maxima
- **THEN** the result's `single_picture_header_flag`, `filter.single_picture_header_flag`,
  and `ccso.single_picture_header_flag` SHALL be `true`, and `max_frame_width` /
  `max_frame_height` SHALL equal the supplied maxima.

#### Scenario: The constructor backs the frame-header round-trips

- **WHEN** the `base_seq()` test helper delegates to the constructor
- **THEN** the existing frame-header-core parse/write round-trip suites SHALL remain
  green (the constructor's view serializes and reparses as before).

#### Scenario: The bridge does not produce packets

- **WHEN** the minimal-intra `CoreSeqView` constructor is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a `FrameHeaderCore` constructor, a
  tile-group OBU, a frame, a packet, or Baseline Encoder Profile v1 output from the
  constructor alone.
