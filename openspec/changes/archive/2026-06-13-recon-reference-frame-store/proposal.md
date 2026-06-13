## Why

The decoder mission needs encoder-reusable reference-frame state before any
closed-loop reconstruction or future encoder roundtrip can be credible.
`splot-recon` already owns immutable decoded frame types, but it has no runtime
container for AV2 § 7.23 reference-frame storage.

## What Changes

- Add Feature ID `RECON-REFERENCE-FRAME-STORE`.
- Add a dependency-free `splot-recon` reference-frame-store API for immutable
  caller-owned frame payload values.
- Provide typed reference slot identifiers, bounded store capacity, slot
  validation, replacement, clearing, immutable lookup, occupancy, and ordered
  iteration.
- Update decoder support docs and feature tracking so the existing
  `reference-frame-store` row moves from `todo` to `supported` for the runtime
  model only.
- Keep byte-consuming decode, reference refresh semantics, frame selection,
  motion compensation, AVM/dav2d integration, and `decode/resource-limit`
  emission out of scope.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: record and test the source-backed reference-frame-store
  runtime model for future decoder and encoder reuse.

## Impact

- Code: `crates/splot-recon`.
- Docs/status: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`,
  generated decoder/feature/spec-coverage status, and
  `docs/IMPLEMENTATION-MATRIX.toml`.
- OpenSpec: `openspec/specs/decoder-support/spec.md`.
- Dependencies: no new crates, no new `splot-*` crate edges, no AVM/dav2d code,
  wrappers, scripts, or CI hooks.
