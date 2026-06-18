# Encoder gap audit

`status: active`  
`owner: encoder`  
`Feature ID: DOC-ENCODER-PROGRAM-CONTRACT`  
`audit date: 2026-06-18`

This audit records the baseline for the first encoder contract PR. It is scoped to
planning and status; it does not claim encoder behavior exists.

## API and CLI baseline

- `splot-encode` is an API shell. `Context::send_frame`,
  `Context::receive_packet`, and `Context::flush` return
  `Error::Unimplemented`.
- `Frame` is an empty placeholder under `ENC-Y4M-INPUT`; `Packet` is only a byte
  buffer wrapper.
- `EncoderConfig` exposes `BitDepth::Twelve`, but current Baseline Encoder Profile
  v1 does not support 12-bit encode.
- The CLI encode command constructs a context and exits with the existing
  "not yet implemented" path. It does not read input or write output.

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
- `splot-encode` does not depend on `splot-recon`. That decision is reserved for
  `encoder-recon-dependency`.

## Conformance baseline

- `splot validate` is the first legality gate for future encoder output.
- `splot decode` evidence is required before public success for closed-loop output,
  but the exact phase boundary is still future work.
- Live AVM/dav2d differential runs are supplemental until self-contained harness
  work exists. They must not make CI depend on the network or uncommitted tools.
- `CONF-AVM-DIFF-HARNESS` remains future work.

## Active ownership baseline

As of 2026-06-18, open PR #234 owns decoder-side coefficient `coeff_br` context
derivation. It touches `docs/IMPLEMENTATION-MATRIX.toml`, but not encoder crate
code, encoder docs, or encoder OpenSpec files. The encoder contract PR should keep
matrix edits to docs/encoder rows and avoid decoder support files.

A local non-open writer-coverage worktree was observed touching generated
writer/status docs and writer-coverage automation. If that work lands first, rebase
the encoder contract branch and rerun the full gates before review.

## Parked work

`toy-intra-encoder-v0` remains unchecked and parked. It is superseded as the
starting point for implementation by Baseline Encoder Profile v1. Future all-intra
work must be proposed with current writer, recon, validation, and conformance gates.
