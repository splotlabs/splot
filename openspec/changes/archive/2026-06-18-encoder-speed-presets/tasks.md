## 1. Runtime API

- [x] 1.1 Add a public `SpeedPreset` type with default, accepted range constants, conversion, and display/error support.
- [x] 1.2 Store `SpeedPreset` in `EncoderRuntimeConfig` with constructors/accessors that preserve the existing thread-count API.
- [x] 1.3 Expose the selected preset through `Context` without adding packet output.

## 2. CLI Integration

- [x] 2.1 Parse `splot encode --speed` through the typed `SpeedPreset` library API.
- [x] 2.2 Add CLI tests for accepted and rejected speed values while preserving the not-implemented encode exit behavior.

## 3. Tracking And Verification

- [x] 3.1 Update `ENC-SPEED-PRESETS` matrix/docs evidence and regenerate generated status docs.
- [x] 3.2 Run focused encoder/CLI tests, OpenSpec validation, feature-status checks, and `cargo xtask ci`.
