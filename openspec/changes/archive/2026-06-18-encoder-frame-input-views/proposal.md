## Why

The encoder API still exposes an empty `Frame` placeholder, so callers cannot
submit real pixel input or validate media-buffer boundaries before future encode
stages retain frames. This change advances `ENC-Y4M-INPUT` with the smallest
useful borrowed-frame surface: validated 8-bit YUV420 input views with explicit
identity and timing metadata, without adding a Y4M parser or any coded output.

## What Changes

- Replace the empty `Frame` placeholder with a validated borrowed-frame input
  model over caller-owned sample buffers.
- Support the Baseline Encoder Profile v1 input subset of 8-bit YUV420 luma and
  chroma planes, including odd visible luma dimensions and derived chroma sizes.
- Model visible dimensions, per-plane stride and buffer length, bit depth,
  chroma format, frame identity, and optional timestamp metadata using typed API
  values.
- Validate plane count, chroma dimensions, stride, visible rows, buffer
  truncation, and arithmetic before any access.
- Define explicit retained input via a shared frame handle for future lookahead,
  with no hidden pixel clone or clone-on-write.
- Keep `send_frame`, `receive_packet`, and `flush` unavailable for successful
  public encoding until a later state-machine and coded-frame path lands.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `encoder-api`: define the encoder frame input view contract and its current
  send-frame interaction with the still-unimplemented encoder lifecycle.
- `runtime`: extend the view-first media-buffer policy to the encode-facing
  retained input path.

## Impact

- Affected Feature ID: `ENC-Y4M-INPUT`.
- Affected code: `crates/splot-encode/src/*`, focused on frame input types and
  tests, plus the thin `splot encode` stub call site.
- Affected docs/specs: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  ledgers, and OpenSpec deltas for `encoder-api` and `runtime`.
- Dependencies: no new third-party dependency and no new crate edge; this builds
  on the existing approved `splot-encode -> splot-recon` edge.
- Validator impact: none. This change does not parse, validate, or emit AV2
  bitstreams and introduces no validator diagnostics.
