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
