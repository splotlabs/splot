# Encoder gap audit

`status: active`
`owner: encoder`
`Feature ID: DOC-ENCODER-PROGRAM-CONTRACT`
`audit date: 2026-06-18`

This audit records the baseline for the first encoder contract PR. It is scoped to
planning and status; it does not claim encoder behavior exists.

## API and CLI baseline

- `splot-encode` is an API shell. `Frame` models validated borrowed 8-bit YUV420
  input views under `ENC-Y4M-INPUT`, and `Context` now exposes a typed
  accepting/draining/finished/failed lifecycle under
  `ENC-CONTEXT-STATE-MACHINE`.
- `Packet` is still only a byte buffer wrapper, and no coded packet production
  path exists.
- `EncoderConfig` exposes `BitDepth::Twelve`, but current Baseline Encoder Profile
  v1 does not support 12-bit encode.
- The CLI encode command constructs a context, exercises the lifecycle boundary,
  and exits with the existing "not yet implemented" path. It does not read input
  or write output.

## Writer baseline

- `ENC-BITSTREAM-WRITER` is the current writer foundation.
- `splot-core` has writer primitives, OBU payload writers for the parsed OBU model,
  Annex B framing helpers, IVF helpers, and round-trip/fuzz coverage.
- This is still syntax/framing support, not an encoder. Coded tile payload
  generation is missing because `RangeEncoder` and the AV2 `decode_tile()` body
  path remain unimplemented.
- Inter first-group tile-group composition remains blocked on inter frame-header
  writer support.
- Partial or unimplemented syntax models must be rejected by writers rather than
  silently emitted.

## Reconstruction baseline

- `splot-recon` exposes frame/plane views, current-frame workspace, intra
  prediction primitives, dequant, inverse transforms, residual addition,
  reference-store pieces, hash input, and Y4M output pieces.
- It is not a byte-consuming decoder and does not yet provide a full encoder
  closed-loop reconstruction API.
- `splot-encode` has a direct `splot-recon` dependency and uses recon borrowed
  plane/shared-frame views for input. It still has no closed-loop reconstruction
  integration, packet generation, or public encode success path; those decisions
  remain future work.

## Conformance baseline

- `splot validate` is the first legality gate for future encoder output.
- `splot decode` evidence is required before public success for closed-loop output,
  but the exact phase boundary is still future work.
- Live AVM/dav2d differential runs are supplemental until self-contained harness
  work exists. They must not make CI depend on the network or uncommitted tools.
- `CONF-AVM-DIFF-HARNESS` remains future work.

## Active ownership baseline

As of the post-merge readiness pass on 2026-06-18, PR #237 has merged into
`main` as `d070ca2a`, and this branch has merged current `main`. No sibling PR is
open against `main`. The earlier `docs/IMPLEMENTATION-MATRIX.toml` same-file
intersection with PR #237 is now part of the base, and no semantic overlap remains.

A local non-open writer-coverage worktree was observed during the initial audit
touching generated writer/status docs and writer-coverage automation. If that work
lands first, merge current `main` into the encoder contract branch and rerun the
full gates before review.

## Parked work

`toy-intra-encoder-v0` remains unchecked and parked. It is superseded as the
starting point for implementation by Baseline Encoder Profile v1. Future all-intra
work must be proposed with current writer, recon, validation, and conformance gates.
