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
