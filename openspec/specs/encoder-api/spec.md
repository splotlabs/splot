# encoder-api Specification

## Purpose

The shape of the future AV2 encoder API in `splot-encode`: bitstream-affecting
configuration plus a push/pull `Context`. No encoding is implemented; every
operation returns `Error::Unimplemented`.

Tracked by Feature IDs: `ENC-Y4M-INPUT`, `ENC-SPEED-PRESETS`,
`ENC-BITSTREAM-WRITER` (the writer this API will drive).
## Requirements
### Requirement: bitstream-affecting vs runtime configuration

`EncoderConfig` SHALL describe only what is encoded (dimensions, bit depth, chroma
format, and future profile/level/color). Runtime/policy knobs (thread count, speed
presets) SHALL NOT live in `EncoderConfig`; they are passed to `Context::new`.

#### Scenario: thread count is not bitstream config

- **WHEN** a caller sets the worker-thread count
- **THEN** it is passed to `Context::new`, not stored in `EncoderConfig`

### Requirement: push/pull state machine

The encoder SHALL expose `send_frame` / `receive_packet` / `flush`. Until the
encoder is implemented, each SHALL return `Error::Unimplemented`, never a panic.

#### Scenario: unimplemented today

- **WHEN** `send_frame`, `receive_packet`, or `flush` is called
- **THEN** `Error::Unimplemented` is returned

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

### Requirement: Baseline profile public support gate

The future encoder public success path SHALL initially be limited to Baseline
Encoder Profile v1 inputs and outputs: 8-bit and 10-bit YUV420 Y4M input and raw
Annex B or IVF output. Twelve-bit input, monochrome, YUV422, YUV444, alpha, live
capture, and non-Y4M input SHALL remain unsupported until separate Feature IDs,
tests, and proof exist.

#### Scenario: unsupported format remains outside the first success path

- **WHEN** a caller requests an input format outside Baseline Encoder Profile v1
- **THEN** the encoder returns an unsupported or unimplemented result
- **AND** the format is not documented as a supported encoder path.

### Requirement: Deterministic runtime policy

The encoder SHALL produce deterministic encoded bytes and structured diagnostics
for any future supported path given the same input, bitstream-affecting
configuration, speed preset, and seed-free runtime policy, independent of the
chosen worker thread count. Thread count SHALL remain runtime policy and SHALL NOT
become bitstream-affecting configuration.

#### Scenario: thread count does not change output

- **WHEN** the same supported input is encoded with one worker and with multiple
  workers
- **THEN** the resulting bytes and diagnostics are identical.

### Requirement: Public encode success requires proof

`send_frame`, `receive_packet`, and `flush` SHALL NOT expose a successful public
encode path until the relevant Feature IDs record writer, validation, tests, and
decode or differential proof in `docs/IMPLEMENTATION-MATRIX.toml`.

#### Scenario: unproven path stays unavailable

- **WHEN** a future encoder path lacks matrix proof for legal stream production
- **THEN** the public API reports unimplemented or unsupported status instead of
  returning a successful packet.
