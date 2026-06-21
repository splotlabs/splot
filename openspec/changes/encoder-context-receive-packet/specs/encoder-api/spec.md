## MODIFIED Requirements

### Requirement: push/pull state machine

The encoder SHALL expose `send_frame` / `receive_packet` / `flush` as a
deterministic push/pull lifecycle. The lifecycle SHALL use explicit accepting,
draining, finished, and failed states, SHALL return operation-specific statuses
for normal flow control, and SHALL return typed encoder state errors for invalid
transitions. `receive_packet` SHALL return a real coded packet (one access unit)
only for the input subset the minimal encoder can encode losslessly (tracked by
`ENC-CONTEXT-RECEIVE-PACKET`), and SHALL return no packet for any other input;
the lifecycle SHALL NOT emit fake packets or report public bitstream production it
did not perform. A successful `send_frame` or `flush` alone SHALL NOT be
documented as successful AV2 encoding.

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

#### Scenario: frame metadata must match config

- **WHEN** a caller sends a frame whose dimensions, bit depth, or chroma layout
  do not match the context configuration
- **THEN** `send_frame` rejects the frame with a typed input-frame error
- **AND** the frame is not queued

#### Scenario: an encodable frame yields a real packet

- **WHEN** a caller sends a frame the minimal encoder can encode losslessly (a
  64x64 frame whose every visible sample is the 128 no-neighbour DC predictor)
  and then flushes
- **THEN** `receive_packet` returns a real coded packet carrying one access unit
- **AND** a later `receive_packet` reports finished

#### Scenario: flush drains unencodable input without fake packets

- **WHEN** a caller sends one or more frames outside the encodable subset and then
  flushes
- **THEN** the context enters draining state
- **AND** repeated `receive_packet` calls retire the queued input without emitting
  a packet until the context reports finished

#### Scenario: terminal state rejects new input

- **WHEN** a context is draining, finished, or failed
- **THEN** `send_frame` fails with a typed encoder state error
- **AND** no input frame is accepted

## REMOVED Requirements

### Requirement: Push/pull lifecycle remains unavailable

**Reason**: `receive_packet` now returns a real coded packet for the
losslessly-encodable input subset (the new `ENC-CONTEXT-RECEIVE-PACKET`
requirement under `encoder-tools`), so the lifecycle is no longer "unavailable".
The still-valid guarantees this requirement carried — frame metadata must match
config, `send_frame`/`flush` alone is not AV2 encoding, and a fully drained
stream returns no further packet — are absorbed into the modified **push/pull
state machine** requirement above.
