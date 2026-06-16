## Context

`splot-recon` owns the source-backed Y4M writer for already materialized
`DecodedFrame<T>` values. The writer serializes stream headers, per-frame
headers, and visible plane samples for supported bit-depth/pixel-format
combinations. Existing unit tests cover exact byte output and error paths, while
the minimal runtime Y4M path covers one byte-consuming decode fixture.

This change adds fuzz coverage for the serialization surface itself. It does
not parse AV2 bytes or create new runtime decode support. The fuzzer input is
used only as a compact description of bounded decoded-frame geometry, format,
sample values, and optional writer behavior.

## Goals / Non-Goals

**Goals:**

- Add `recon_y4m_output_bytes` to the fuzz crate.
- Build only valid, small `DecodedFrame<u8>` or `DecodedFrame<u16>` values.
- Exercise all currently supported Y4M tags: mono, 4:2:0, 4:2:2, and 4:4:4 at
  8-bit and 10-bit depths.
- Exercise stream header writing, frame header writing, visible-plane payload
  writing, multi-frame matching writes, and typed stream/frame mismatch returns.
- Keep allocations and output buffers bounded.

**Non-Goals:**

- Fuzzing AV2 bitstream decode or `DecodeContext::decode_y4m_bytes`.
- Generating or committing a corpus.
- Filesystem publication, CLI temp-file behavior, raw output, hash report
  output, AVM/dav2d/ffmpeg, networking, subprocesses, or new dependencies.
- Claiming broad runtime Y4M support beyond existing minimal-tier runtime rows.

## Decisions

1. Fuzz `splot-recon` directly.

   Rationale: `Y4mWriter` is a library serialization boundary over typed decoded
   frames. Driving it through `DecodeContext` would mostly fuzz the already
   covered minimal decode tier and would not vary frame formats.

2. Build only valid frames.

   Rationale: The target is for output serialization, not constructor invariant
   testing. Geometry, chroma sizes, storage lengths, sample ranges, and crop
   alignment should be normalized before calling constructors so fuzzer time is
   spent inside the writer.

3. Bound dimensions and frame count aggressively.

   Rationale: CI fuzz smoke must remain quick and resource-stable. Small
   dimensions still exercise crop/stride/padding, chroma subsampling, 8-bit
   one-byte writes, and 10-bit little-endian writes.

4. Treat writer errors as typed results.

   Rationale: The no-panic invariant is the product. The target may exercise a
   writer that fails after a bounded byte budget; `Y4mError::Io` is an expected
   typed result.

## Risks / Trade-offs

- [Risk] The new row is mistaken for runtime Y4M decode coverage.
  Mitigation: name it as `recon`/serialization fuzz and keep runtime decode
  claims in `decode-y4m-runtime-output`.

- [Risk] Structured generation filters too much input before serialization.
  Mitigation: normalize bytes into valid frames instead of rejecting most
  inputs.

- [Risk] Fuzz output buffers grow large.
  Mitigation: cap visible luma dimensions, frame count, stride padding, and
  writer byte budgets.
