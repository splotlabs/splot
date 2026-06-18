## MODIFIED Requirements

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
