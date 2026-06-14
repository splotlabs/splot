## Why

`splot-recon` now has caller-supplied decoded frame models and deterministic
frame hashes, but it still cannot serialize those modeled frames to the future
Y4M output format. A source-backed Y4M writer is the next encoder-grade
reconstruction artifact because it lets future decoder and encoder roundtrip
work share one visible-sample output path without making `splot decode` succeed
before runtime reconstruction exists.

## What Changes

- Add Feature ID `RECON-Y4M-OUTPUT-WRITER`.
- Add a `splot-recon` Y4M writer API for caller-supplied `DecodedFrame<T>`
  values.
- Write a Y4M stream header, per-frame `FRAME` headers, and visible decoded
  samples in Y/U/V plane order while excluding stride and coded padding.
- Support the modeled AV2 8-bit and 10-bit monochrome, 4:2:0, 4:2:2, and
  4:4:4 output formats through pinned repository-owned Y4M chroma tags.
- Reject invalid frame rates, unsupported frame formats, and stream/frame
  parameter mismatches with typed writer errors before writing frame payloads.
- Update decoder roadmap, decoder support matrix/status, implementation matrix,
  and OpenSpec specs to distinguish source-backed library Y4M writing from
  future byte-consuming `splot decode -o` runtime output.
- Keep byte-consuming decode, tile payload parsing, reconstruction algorithms,
  output scheduling, film-grain synthesis, AVM/dav2d execution, and CLI Y4M
  success out of scope.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: add source-backed Y4M writing for caller-supplied decoded
  frames while keeping runtime `splot decode` Y4M output unsupported.

## Impact

- Code/API: `crates/splot-recon` gains a Y4M writer module and tests.
- Dependencies: no new crate dependency; implementation uses `std::io::Write`.
- Docs/status: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder status,
  `docs/IMPLEMENTATION-MATRIX.toml`, and generated feature/spec status as
  required by repo gates.
- OpenSpec: `openspec/specs/decoder-support/spec.md`.
- Boundary: no AVM/dav2d source, snippets, binaries, submodules, dependencies,
  wrappers, build probes, scripts, CI jobs, runtime process execution, local
  absolute paths, or mandatory reference-tool tests.
