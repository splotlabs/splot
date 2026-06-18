## ADDED Requirements

### Requirement: Borrowed frame input views

The encoder API SHALL expose a `Frame` input model backed by borrowed 8-bit
YUV420 luma and chroma plane views, tracked by `ENC-Y4M-INPUT`. A constructed
frame SHALL carry typed frame identity, optional timestamp ticks, visible luma
size, bit depth, chroma layout, and per-plane stride/visible-rectangle metadata.
Construction SHALL validate plane view geometry and SHALL NOT allocate or copy
sample data.

#### Scenario: valid odd-size YUV420 input is accepted

- **WHEN** a caller constructs a frame for a 3x5 8-bit YUV420 picture with valid
  Y, U, and V borrowed buffers and sufficient strides
- **THEN** the frame is accepted without copying sample data
- **AND** the derived U and V visible sizes are 2x3
- **AND** visible-row iteration exposes only visible samples, excluding stride
  padding

#### Scenario: truncated plane is rejected

- **WHEN** a caller constructs a frame whose visible rectangle and stride require
  more samples than the borrowed backing buffer contains
- **THEN** construction fails with a typed encoder error
- **AND** the caller receives the failing plane identity

#### Scenario: unsupported input format is rejected

- **WHEN** a caller constructs a frame whose metadata requests any format other
  than 8-bit YUV420
- **THEN** construction fails with a typed unsupported-input error
- **AND** no successful public encode path is exposed

#### Scenario: invalid plane count is rejected

- **WHEN** 8-bit YUV420 frame metadata is provided without both chroma planes
- **THEN** construction fails with a typed missing-plane error

### Requirement: Push/pull lifecycle remains unavailable

The encoder context SHALL accept the real frame input type at the `send_frame`
boundary, but `send_frame`, `receive_packet`, and `flush` SHALL continue to
return `splot_core::Error::Unimplemented` until the encoder state-machine and a
proved coded-frame path land under separate Feature IDs.

#### Scenario: send frame remains unimplemented

- **WHEN** a caller sends a valid borrowed input frame to `Context::send_frame`
- **THEN** the call returns `splot_core::Error::Unimplemented`
- **AND** no packet or fake encode success is produced
