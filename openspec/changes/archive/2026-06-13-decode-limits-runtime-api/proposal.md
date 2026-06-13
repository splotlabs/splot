## Why

The decoder roadmap already requires callers to provide resource limits before
future byte-consuming decode code allocates from bitstream-derived values, but
`splot-decode` still has no runtime type API for that contract. The next slice
should make `DecodeOptions { limits: DecodeLimits }` source-backed with finite
CI-safe defaults before any decode path can rely on the contract.

Feature ID: `DECODE-LIMITS-RUNTIME-API`.

## What Changes

- Add a dependency-free `splot-decode` runtime API for `DecodeOptions`,
  `DecodeLimits`, typed limit names, typed units, finite defaults, and pure
  check helpers.
- Preserve the current unsupported `splot decode` behavior: no input reads, no
  output writes, no CLI behavior change, and no byte-consuming decode path.
- Keep `decode/resource-limit` planned but unemitted. The new limit violation
  value is not a `DecodeDiagnostic` and does not carry a decoder rule id,
  severity, spec section, matrix row, byte offset, bit offset, or remediation.
- Update decoder support docs, implementation matrix status, generated status
  docs, and OpenSpec requirements to show that the limits contract now has a
  runtime API while enforcement remains unintegrated.
- Avoid new dependencies and avoid any AVM/dav2d source, wrappers, tooling,
  scripts, CI, fixtures, or executable reference integration.

## Capabilities

### New Capabilities

### Modified Capabilities

- `decoder-support`: the decode limits requirement changes from documentation
  only to a source-backed `splot-decode` runtime API for configured thresholds
  and pure checks, while byte-consuming enforcement and the
  `decode/resource-limit` diagnostic remain future work.

## Impact

- Affected crate: `crates/splot-decode`.
- Affected docs/status: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/IMPLEMENTATION-MATRIX.toml`,
  generated status docs, and OpenSpec `decoder-support`.
- Affected automation: existing `cargo xtask check-diagnostic-registry`,
  `cargo xtask check-dependency-direction`, `cargo xtask check-decoder-support`,
  and `cargo xtask check-feature-status` continue to gate the change.
- No dependency graph change, CLI behavior change, emitted decoder diagnostic,
  byte-consuming decode path, frame hash, Y4M output, reconstruction behavior,
  reference-frame-store behavior, AVM/dav2d integration, scripts, or CI changes
  are included.
