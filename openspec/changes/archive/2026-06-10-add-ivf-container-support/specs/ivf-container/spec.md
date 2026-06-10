## ADDED Requirements

### Requirement: IVF header parsing

`splot-core` SHALL parse the IVF `DKIF` file header as a container header without
using AV2 syntax assumptions. The parser SHALL expose the signature, version,
header length, fourcc, width, height, timebase denominator, timebase numerator,
declared frame count, and unused field as typed data.

#### Scenario: Valid IVF header

- **WHEN** an input begins with a complete 32-byte IVF header whose signature is
  `DKIF`
- **THEN** the parser SHALL return a typed header
- **AND** SHALL record the payload start offset after the declared header length.

#### Scenario: Invalid or truncated IVF header

- **WHEN** an IVF header is truncated, has a non-`DKIF` signature, or declares a
  header length smaller than 32 bytes
- **THEN** the parser SHALL return a typed `IvfError`
- **AND** SHALL NOT panic.

### Requirement: IVF frame parsing

`splot-core` SHALL parse IVF frame records as a little-endian 32-bit payload size,
a little-endian 64-bit presentation timestamp, and exactly that many payload bytes.
Frame payload bytes SHALL remain opaque to the IVF parser.

#### Scenario: Valid IVF frame

- **WHEN** a complete frame record follows a valid IVF header
- **THEN** the parser SHALL expose the frame payload, its presentation timestamp,
  and its byte offset in the original input.

#### Scenario: Truncated IVF frame

- **WHEN** a frame record or declared frame payload is truncated
- **THEN** the parser SHALL retain previously parsed frames
- **AND** SHALL return a typed `IvfError` carrying the failing byte offset.

### Requirement: IVF writing

`splot-core` SHALL expose panic-free helpers to write IVF headers and frame records
for caller-supplied fourcc, dimensions, timebase, frame count, presentation
timestamps, and payload bytes.

#### Scenario: Writer error propagation

- **WHEN** the output writer returns an I/O error while writing an IVF header or
  frame
- **THEN** the IVF writer SHALL return that error to the caller
- **AND** SHALL NOT panic.

### Requirement: IVF/Annex B input detection

`splot-core` SHALL provide an input-format parser that detects `DKIF` as IVF and
treats all other inputs as raw Annex B. For IVF, each frame payload SHALL be parsed
as AV2 Annex B OBUs while preserving byte offsets relative to the original input.

#### Scenario: IVF-wrapped Annex B OBUs

- **WHEN** an IVF stream contains frame payloads with valid Annex B OBUs
- **THEN** the input parser SHALL return an IVF stream with parsed frames
- **AND** SHALL expose the underlying OBU envelopes in frame order with original
  byte offsets.

#### Scenario: Raw Annex B compatibility

- **WHEN** an input does not start with `DKIF`
- **THEN** the input parser SHALL parse it as raw Annex B
- **AND** existing raw bitstream behavior SHALL be preserved.
