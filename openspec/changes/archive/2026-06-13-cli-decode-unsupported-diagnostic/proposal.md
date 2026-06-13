## Why

`splot decode` currently exits as a generic "not yet implemented" stub, which
does not satisfy the decoder mission requirement that unsupported decode paths
fail explicitly and structurally. Replacing the stub with a stable unsupported
diagnostic lets users and future encoder tests distinguish intentional decoder
absence from operational failures before any decoder crate is approved.

## What Changes

- Add a structured `decode/unsupported-feature` diagnostic for the existing
  `splot decode` CLI entry point.
- Support text and JSON rendering for the diagnostic, with exit code `1` for
  intentional unsupported decode and exit code `2` reserved for operational
  errors.
- Cite AV2 § 7.1, Feature ID `CLI-DECODE`, and decoder support row
  `cli-decode-entrypoint` in the diagnostic payload.
- Update decoder support docs, matrix rows, generated status, and feature status
  to reflect the implemented unsupported diagnostic.
- Add self-contained CLI tests for text output, JSON output, and no file
  creation while decode remains unsupported.
- Do not add a decoder crate, reconstruction crate, dependency-graph change,
  AVM/dav2d integration, fixture generator, or pixel output.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: the structured unsupported-feature diagnostic contract
  becomes implemented for the `splot decode` CLI entry point.

## Impact

- Affected code: `crates/splot-cli/src/commands/decode.rs` and CLI tests.
- Affected docs/status: decoder roadmap/support matrix/status, implementation
  matrix, generated feature/spec coverage, and OpenSpec decoder-support specs.
- Dependencies: none.
- Runtime boundary: no AVM/dav2d lookup, invocation, wrapper, dependency, or CI
  step is introduced.
