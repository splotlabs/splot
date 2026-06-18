# Encoder goal

`status: active`
`owner: encoder`
`Feature ID: DOC-ENCODER-PROGRAM-CONTRACT`

This document defines the program target for the first supported encoder profile.
It is not an implementation claim: today `splot-encode` still returns
`Error::Unimplemented` for encode operations.

## Baseline Encoder Profile v1

Baseline Encoder Profile v1 is the first public success target for `splot encode`
and the `splot-encode` library:

- Input: finite YUV4MPEG2 (`.y4m`) streams with 8-bit or 10-bit YUV420 pictures.
- Output: raw Annex B AV2 bitstreams and IVF-wrapped AV2 bitstreams, using shared
  `splot-core` container writers.
- Coding order: all-intra legal streams first, then basic inter streams after
  reference-state and closed-loop proof exist.
- Reconstruction: closed-loop reconstruction is required before any public success
  path is documented as supported.
- Determinism: identical input, bitstream configuration, speed preset, and runtime
  policy must produce identical bytes and diagnostics across supported thread
  counts.
- Evidence: every supported path needs matrix proof, parser/writer tests,
  `splot validate` evidence, `splot inspect` or structured diagnostic evidence
  when relevant, and decode or differential evidence appropriate for the phase.
- CI: proof must be self-contained in CI. External AVM or dav2d runs can be local
  supplemental evidence, but CI cannot depend on network fetches or uncommitted
  external binaries.

## Out of profile

These are deliberately out of Baseline Encoder Profile v1 unless separate Feature
IDs, OpenSpec changes, tests, and proof land:

- 12-bit encode support.
- Monochrome, YUV422, YUV444, alpha, RGB, live capture, or non-Y4M input.
- Public lossy quality claims, perceptual tuning, scene-cut decisions, lookahead,
  two-pass rate control, or production speed benchmarking.
- Streaming network output.
- External codec integration or copied reference-code tables, constants, comments,
  or prose.

## Current truth

The current encoder truth is intentionally narrow:

- `splot-encode` owns API shape only. `send_frame`, `receive_packet`, and `flush`
  return `Error::Unimplemented`.
- `splot-core` owns writer primitives, OBU payload writers, and Annex B/IVF helpers,
  but it does not generate entropy-coded tile payloads.
- `splot-recon` owns reconstruction building blocks, and `splot-encode` has a
  private dependency boundary for future reuse; no public recon-backed encoder
  API or closed-loop integration exists yet.
- The parked `toy-intra-encoder-v0` change is superseded as the implementation
  starting point. Future all-intra work must be re-proposed under this profile.

The canonical status remains `docs/IMPLEMENTATION-MATRIX.toml`; this document is a
program contract and roadmap entry point.
