## Why

The local-reference evidence manifest exists but still has no evidence entries.
The decoder roadmap and support matrix already cite free-form AVM/dav2d raw
output agreement for two committed fixtures; moving that metadata into the
checked manifest makes the evidence portable and machine-verifiable without
running external decoders.

## What Changes

- Add two non-executable local-reference evidence entries to
  `docs/LOCAL-REFERENCE-EVIDENCE.toml` for the already-recorded AVM/dav2d raw
  MD5 agreement on committed 8-bit and 10-bit intra IVF fixtures.
- Record repo-relative fixture identity, byte length, SHA-256, sanitized
  reference-tool metadata, output digest IDs, and equality assertions.
- Update decoder docs, support matrix/status, implementation matrix/status, and
  OpenSpec so the manifest is no longer documented as empty.
- Keep AVM/dav2d local-only: no source, binaries, wrappers, scripts, build
  probes, dependencies, CI jobs, or mandatory tests are added.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: the portable local-reference evidence manifest requirement
  gains scenarios for committed evidence entries backed by existing fixture
  bytes and reference digest metadata.

## Impact

- Affected docs/manifests: `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `docs/DECODER-ROADMAP.md`, `docs/DECODER-SUPPORT-MATRIX.toml`, generated
  decoder/feature/spec status docs, and `docs/IMPLEMENTATION-MATRIX.toml`.
- Affected OpenSpec: `decoder-support` delta for checked evidence entries.
- No crate or dependency graph change.
- No validator, decode runtime, reconstruction, hash computation, Y4M output, or
  AV2 conformance behavior change.
- No AVM/dav2d repository integration or CI/runtime invocation.
