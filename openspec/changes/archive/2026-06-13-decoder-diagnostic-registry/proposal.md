## Why

`splot decode` now emits `decode/unsupported-feature`, but decoder diagnostics are
only documented indirectly in the decoder support matrix. The decoder mission
requires every `splot decode` error to have a stable documented code, message,
spec citation when applicable, and support-matrix row; adding a decoder-specific
registry and drift gate makes that contract enforceable before real decode
paths multiply the diagnostic surface.

## What Changes

- Add `DOC-DECODER-DIAGNOSTICS` / `XTASK-DECODER-DIAGNOSTIC-REGISTRY` tracking
  for a decoder diagnostic registry and CI check.
- Add `docs/DECODER-DIAGNOSTICS.md` as the canonical registry for emitted
  `decode/*` rule IDs.
- Extend `cargo xtask check-diagnostic-registry` so it also compares emitted
  decoder rule-id literals against the decoder registry.
- Wire the new registry check into `cargo xtask ci`, docs, generated status, and
  OpenSpec requirements.
- Non-goals: no new decoder crate, no new dependency, no AVM/dav2d integration,
  no real pixel decode, no expanded supported stream tier, and no validator
  diagnostic behavior change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `process`: add a repository process requirement for the decoder diagnostic
  registry and its drift gate.
- `decoder-support`: make documented decoder support include the canonical
  decoder diagnostic registry for emitted `decode/*` rule IDs.

## Impact

- Code: `xtask/src/diagnostic_registry.rs`, `xtask/src/main.rs` help text if
  needed, and unit tests for decoder registry drift.
- Docs/status: `docs/DECODER-DIAGNOSTICS.md`, `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-SUPPORT-STATUS.md`,
  `docs/FEATURE-TRACKING.md`, `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, and `docs/SPEC-MAPPING.md`.
- APIs/dependencies: no public Rust API change and no dependency graph change.
- Diagnostics: `decode/unsupported-feature` becomes CI-enforced in the decoder
  registry while remaining linked to `cli-decode-entrypoint`.
