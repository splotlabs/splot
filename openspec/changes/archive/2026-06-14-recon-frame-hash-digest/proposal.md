## Why

The decoder roadmap defines `splot-dfh-sha256-v1`, but `splot-recon` currently
only serializes the canonical decoded-frame hash input bytes. A source-backed
digest API is the next encoder-grade reconstruction artifact because future
decoder fixtures and encoder roundtrip tests need a stable frame identity before
Y4M output or full pixel reconstruction is useful.

## What Changes

- Add Feature ID `RECON-FRAME-HASH-DIGEST`.
- Add a `splot-recon` API that computes the repository-owned
  `splot-dfh-sha256-v1` digest for a caller-supplied `DecodedFrame<T>` by
  hashing the existing `DecodedFrameHashInput` byte stream.
- Expose stable algorithm, byte-stream, and variant identifiers plus lowercase
  hex formatting for fixture/manifests and future CLI hash output.
- Update decoder roadmap, decoder support matrix/status, implementation matrix,
  and OpenSpec specs to distinguish supported `splot` digest computation from
  future AV2 metadata MD5 verification and runtime decode output.
- Keep byte-consuming decode, tile payload parsing, reconstruction algorithms,
  Y4M output, AVM/dav2d execution, and CLI hash output support out of scope.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: promote the deterministic decoded-frame hash contract from
  source-backed input serialization to source-backed `splot-dfh-sha256-v1`
  digest computation over caller-supplied decoded frames.

## Impact

- Code/API: `crates/splot-recon` hash API and tests.
- Dependencies: likely add the existing workspace `sha2` dependency to
  `splot-recon`; no new third-party crate is introduced, no `splot-*` dependency
  edge is added, and `splot-recon` remains scheduler-free.
- Docs/status: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder status,
  `docs/IMPLEMENTATION-MATRIX.toml`, and generated feature/spec status as
  needed.
- OpenSpec: `openspec/specs/decoder-support/spec.md`.
- Boundary: no AVM/dav2d source, snippets, binaries, submodules, dependencies,
  wrappers, build probes, scripts, CI jobs, runtime process execution, or
  mandatory tests.
