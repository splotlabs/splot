## Why

`splot decode` still ignores its input path and always renders the same
unsupported diagnostic. The decoder mission now has a bounded raw-byte planner,
so the CLI should hand real input bytes to `DecodeContext::plan_bytes` and fail
with source-aware structured diagnostics until runtime decode output exists.

## What Changes

- Change `splot decode` to read the requested input bytes before touching any
  output path and to run `DecodeContext::plan_bytes` with finite default
  `DecodeOptions`.
- Add library-owned diagnostic descriptors for decode planner failures:
  malformed source, resource-limit failures, unsupported structures, and
  plan-success-but-runtime-unsupported.
- Render those diagnostics from the CLI in the existing text/JSON shape, with
  stable rule IDs, spec/matrix/feature fields, and no output writes.
- Update the decoder diagnostic registry, CLI tests, decoder support docs,
  implementation matrix, and OpenSpec requirements for Feature IDs
  `CLI-DECODE`, `DECODE-BYTE-STREAM-PLANNER`,
  `DECODE-STREAM-STATE-PLANNER`, and `DOC-DECODE-LIMITS-CONTRACT`.
- Keep runtime tile decode, symbol/CDF decode, reconstruction, frame-hash
  digest, Y4M output, reference refresh, AVM/dav2d integration, and new
  dependencies out of scope.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: adds the CLI byte-planner handoff requirement, source-aware
  decode diagnostics, and no-output/no-external-decoder constraints for the
  first non-generic `splot decode` behavior.

## Impact

- Affected code: `crates/splot-cli/src/commands/decode.rs`,
  `crates/splot-cli/tests/decode_cli.rs`, `crates/splot-decode/src`,
  `docs/DECODER-DIAGNOSTICS.md`, decoder support docs/matrices, and OpenSpec
  decoder-support specs.
- Public API: `splot-decode` gains diagnostic adapter functions/types for
  converting `DecodeError` and plan-success runtime deferral into
  `DecodeDiagnostic` values. Existing `DecodeContext::plan_bytes` remains the
  byte-consuming planner API.
- Dependencies: no new third-party dependency, no new production crate edge,
  and no AVM/dav2d source, wrapper, build, script, CI, or runtime process use.
- Diagnostics: move `decode/resource-limit` from planned to emitted and add
  emitted source-aware decode diagnostics as needed for malformed planner
  failures while preserving `decode/unsupported-feature` for unsupported
  structures and runtime decode deferral.
