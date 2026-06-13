## Why

The decoder roadmap defines a deterministic decoded-frame hash contract, but
`splot-recon` cannot yet produce the canonical sample-byte stream that a digest
will consume. Adding the byte serialization first gives future decoder and
encoder roundtrip work a testable, dependency-free foundation before deciding
where to wire SHA-256.

## What Changes

- Add Feature ID `RECON-HASH-INPUT-SERIALIZATION`.
- Add a dependency-free `splot-recon` API that writes canonical decoded-frame
  hash input bytes from an existing `DecodedFrame<T>`.
- Serialize only visible output samples in Y, U, V order, excluding stride and
  padding.
- Serialize `u8` samples as one byte and `u16` samples as little-endian two-byte
  values according to the frame bit depth.
- Keep SHA-256 digest computation, metadata MD5 verification, byte-consuming
  decode, Y4M output, AVM/dav2d integration, and new dependencies out of scope.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: source-back the canonical decoded-frame hash input
  serialization portion of the deterministic frame-hash contract while keeping
  digest computation future work.

## Impact

- Code: `crates/splot-recon`.
- Docs/status: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder/feature/spec-coverage
  status, and `docs/IMPLEMENTATION-MATRIX.toml`.
- OpenSpec: `openspec/specs/decoder-support/spec.md`.
- Dependencies: no new crates, no new `splot-*` crate edges, no AVM/dav2d code,
  wrappers, scripts, or CI hooks.
