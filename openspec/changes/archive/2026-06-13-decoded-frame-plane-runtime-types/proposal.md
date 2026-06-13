## Why

The decoder roadmap already defines the decoded frame and plane model, but
`splot-recon` is still an empty scaffold. The next encoder-useful decoder slice
should turn that contract into safe, dependency-free Rust types before any byte
consuming decode path or hash output can rely on frame storage.

Feature ID: `INFRA-RECON-FRAME-PLANE-TYPES`.

## What Changes

- Add the first public `splot-recon` runtime API for decoded output model data:
  bit depth, pixel format, plane identifiers, output indices, dimensions,
  visible rectangles, immutable owned planes, and immutable decoded frames.
- Enforce AV2-derived format, crop, plane-shape, stride, checked arithmetic, and
  sample-range invariants in constructors and unit tests.
- Keep `splot-recon` independent of other `splot-*` crates and avoid new
  third-party dependencies.
- Update decoder support and implementation matrices to show that frame/plane
  types now exist while runtime decode, reconstruction algorithms, hashes, Y4M,
  and reference-frame-store behavior remain unimplemented.
- Archive the OpenSpec change with agent review evidence.

## Capabilities

### New Capabilities

### Modified Capabilities

- `decoder-support`: the decoded frame and plane model requirement changes from
  contract-only to a runtime type API in `splot-recon`, with explicitly
  self-contained tests and no runtime decode claim.

## Impact

- Affected crate: `crates/splot-recon`.
- Affected docs/status: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/IMPLEMENTATION-MATRIX.toml`,
  generated status docs, and OpenSpec `decoder-support`.
- Affected automation: existing `cargo xtask check-dependency-direction`,
  `cargo xtask check-decoder-support`, and `cargo xtask check-feature-status`
  continue to gate the new row.
- No CLI behavior, `splot-decode` runtime behavior, AV2 parser behavior,
  encoder behavior, AVM/dav2d integration, reference fixtures, hash
  computation, Y4M output, or dependency graph change is included.
