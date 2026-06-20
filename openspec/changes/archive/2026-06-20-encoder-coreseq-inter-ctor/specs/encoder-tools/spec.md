## ADDED Requirements

### Requirement: Encoder writer-input minimal-intra inter view constructor

The encoder writer bridge SHALL provide a public `splot-core` constructor for the
all-disabled minimal-intra `CoreSeqInterView`, tracked by `ENC-WRITER-INPUT-INTER-VIEW`,
so an encoder can assemble a `CoreSeqView` for `write_tile_group_obu` without a parsed
`SequenceHeader` (the inter view is the one nested sequence sub-view that is
`#[non_exhaustive]` with crate-private fields). The constructor SHALL return the inert
§ 5.4.6 inter view an intra sequence header signals — every inter tool off and every
motion mode disabled. The three `base_inter()` parser/writer test helpers SHALL delegate
to it so the existing frame-header round-trip suites regress it. It SHALL NOT provide a
`CoreSeqView` / `FrameHeaderCore` constructor, a tile-group OBU, a frame, a packet, or
`Context::receive_packet` output.

#### Scenario: The constructor is the all-disabled inter view

- **WHEN** the minimal-intra inter view constructor is called
- **THEN** every inter tool field SHALL be `false`/`0` and every motion mode SHALL be
  disabled.

#### Scenario: The constructor backs the frame-header round-trips

- **WHEN** the `base_inter()` test helpers delegate to the constructor
- **THEN** the existing frame-header-core parse/write round-trip suites SHALL remain
  green (the constructor's view serializes and reparses as before).

#### Scenario: The bridge does not produce packets

- **WHEN** the minimal-intra inter view constructor is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a `CoreSeqView` / `FrameHeaderCore`
  constructor, a tile-group OBU, a frame, a packet, or Baseline Encoder Profile v1
  output from the constructor alone.
