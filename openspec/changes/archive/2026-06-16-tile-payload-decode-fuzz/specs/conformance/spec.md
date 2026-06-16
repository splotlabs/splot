## ADDED Requirements

### Requirement: Tile payload runtime byte fuzzing

The conformance suite SHALL include a self-contained cargo-fuzz target named
`tile_payload_decode_bytes` that exercises the current minimal runtime tile
payload boundary through a feature-gated `splot-decode` fuzzing harness. The
target SHALL use bounded in-memory tile-payload bytes, SHALL run without
filesystem output, network access, subprocesses, AVM, dav2d, or ffmpeg, and
SHALL accept typed decode errors for malformed or unsupported mutations.

#### Scenario: Tile payload mutations stay panic-free

- **WHEN** `tile_payload_decode_bytes` mutates bounded tile-payload bytes and
  calls the fuzzing harness with finite limits
- **THEN** the call returns a typed success or typed decode error without
  panicking, hanging, writing files, or invoking external tools

#### Scenario: Successful mutation keeps frontier invariants

- **WHEN** a fuzz-generated tile-payload mutation reaches the boundary or
  minimal block-symbol frontier successfully
- **THEN** the target validates only stable boundary/frontier invariants such as
  single-tile work-unit shape, symbol initialization bounds, typed unsupported
  boundary metadata, and successful frontier summary bounds

### Requirement: Tile payload fuzz target remains self-contained in CI

The CI fuzz-smoke job SHALL enumerate and run `tile_payload_decode_bytes` with
the same bounded cargo-fuzz smoke policy as the other targets. Seed corpus setup
SHALL provide a minimal input for the target without adding large corpora or
external fixtures.

#### Scenario: CI discovers the new target

- **WHEN** CI runs the fuzz-smoke job after this change
- **THEN** `cargo +nightly fuzz list` includes `tile_payload_decode_bytes` and
  the job runs it without a hardcoded target subset
