## Why

`splot decode` already emits a stable structured unsupported diagnostic, but the
diagnostic descriptor still lives in the CLI command. Now that `splot-decode`
exists, the decoder crate should own decoder diagnostic data while `splot-cli`
stays a renderer and argument/IO boundary.

Feature ID: `DECODE-UNSUPPORTED-DIAGNOSTIC-API`.

## What Changes

- Add a minimal `splot-decode` public API for the current
  `decode/unsupported-feature` diagnostic descriptor.
- Have `splot-cli` render that library-owned descriptor in text and JSON modes
  without changing exit code, stdout/stderr placement, field values, input reads,
  or output path behavior.
- Allow the `splot-cli -> splot-decode` dependency edge and document that it is
  diagnostic-rendering plumbing, not runtime decode support.
- Update decoder diagnostics docs, decoder support matrix/status,
  implementation matrix/status, OpenSpec, and architecture guidance.

## Capabilities

### New Capabilities

### Modified Capabilities

- `decoder-support`: the existing structured unsupported diagnostic requirement
  now says the descriptor is owned by `splot-decode` and rendered by the CLI.
- `process`: the decoder crate dependency-direction requirement now permits
  `splot-cli -> splot-decode` for library-owned unsupported diagnostics.

## Impact

- Affected crates: `splot-decode` and `splot-cli`.
- Affected automation: `cargo xtask check-dependency-direction` allows the new
  internal edge; `cargo xtask check-diagnostic-registry` continues to enforce
  exactly one emitted `decode/*` rule.
- Affected docs/status: `AGENTS.md`, `docs/ARCHITECTURE.md`,
  `docs/DECODER-DIAGNOSTICS.md`, `docs/DECODER-SUPPORT-MATRIX.toml`, generated
  decoder/feature status docs, implementation matrix, and OpenSpec specs.
- No runtime decode, reconstruction, frame hashing, Y4M output, reference
  evidence, external decoder integration, AVM/dav2d use, or
  `splot-decode -> splot-core/splot-recon` dependency is added.
