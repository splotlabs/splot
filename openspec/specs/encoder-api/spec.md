# encoder-api Specification

## Purpose

The shape of the future AV2 encoder API in `splot-encode`: bitstream-affecting
configuration plus a push/pull `Context`. The lifecycle state machine is
implemented, but no coded packet production or successful public encode path is
implemented.

Tracked by Feature IDs: `ENC-Y4M-INPUT`, `ENC-CONTEXT-STATE-MACHINE`,
`ENC-SPEED-PRESETS`, `ENC-BITSTREAM-WRITER` (the writer this API will drive).
## Requirements
### Requirement: bitstream-affecting vs runtime configuration

`EncoderConfig` SHALL describe only what is encoded (dimensions, bit depth, chroma
format, and future profile/level/color). Runtime/policy knobs (thread count, speed
presets) SHALL NOT live in `EncoderConfig`; they are passed to `Context::new`.

#### Scenario: thread count is not bitstream config

- **WHEN** a caller sets the worker-thread count
- **THEN** it is passed to `Context::new`, not stored in `EncoderConfig`

### Requirement: push/pull state machine

The encoder SHALL expose `send_frame` / `receive_packet` / `flush` as a
deterministic push/pull lifecycle. The lifecycle SHALL use explicit accepting,
draining, finished, and failed states, SHALL return operation-specific statuses
for normal flow control, and SHALL return typed encoder state errors for invalid
transitions. Until a coded-frame path lands, this lifecycle SHALL NOT emit fake
packets or report successful public bitstream production.

#### Scenario: receive before input needs data

- **WHEN** a newly created context receives a packet before any frame is sent
- **THEN** `receive_packet` reports that more input is needed
- **AND** the context remains accepting input

#### Scenario: bounded send backpressure

- **WHEN** callers send frames until the bounded input queue is full
- **THEN** the frame that fills the queue is accepted
- **AND** a later send reports queue-full backpressure without changing state
- **AND** the later send does not consume the borrowed input frame, so the
  caller can retry it after draining

#### Scenario: flush drains queued input without fake packets

- **WHEN** a caller sends one or more valid frames and then flushes
- **THEN** the context enters draining state
- **AND** repeated `receive_packet` calls retire queued input without emitting a
  packet until the context reports finished

#### Scenario: terminal state rejects new input

- **WHEN** a context is draining, finished, or failed
- **THEN** `send_frame` fails with a typed encoder state error
- **AND** no input frame is accepted

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
boundary and SHALL expose a real lifecycle state machine, but `receive_packet`
SHALL continue to return no coded packet until the encoder state-machine and a
proved coded-frame path land under separate Feature IDs. A successful
`send_frame` or `flush` SHALL NOT be documented as successful AV2 encoding.

#### Scenario: send frame is lifecycle success only

- **WHEN** a caller sends a valid borrowed input frame to `Context::send_frame`
- **THEN** the call returns an operation-specific accepted or backpressure status
- **AND** no packet or fake encode success is produced

#### Scenario: frame metadata must match config

- **WHEN** a caller sends a frame whose dimensions, bit depth, or chroma layout
  do not match the context configuration
- **THEN** `send_frame` rejects the frame with a typed input-frame error
- **AND** the frame is not queued

#### Scenario: end of stream has no packet before encode core

- **WHEN** all accepted input has been drained after flush
- **THEN** `receive_packet` reports finished
- **AND** no packet bytes are returned

### Requirement: Encoder lifecycle fuzz coverage

The encoder API SHALL include a bounded fuzz target for arbitrary lifecycle
command sequences over valid borrowed frames. The target SHALL exercise send,
receive, flush, repeated flush, backpressure, end-of-stream, and invalid-state
paths without panicking or emitting packets before packet production is
implemented.

#### Scenario: arbitrary lifecycle commands are bounded

- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it maps them to a finite command sequence over a small valid frame
- **AND** every command returns a typed status or typed error without panicking

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
