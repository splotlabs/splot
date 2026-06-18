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
- `ENC-SYNTAX-IR` adds a private deterministic planning model for future
  sequence/frame/tile/block/token decisions. It is not re-exported, owns no bit
  writer, and does not produce packets.
- `ENC-MINIMAL-HEADER-PLAN` adds a private bridge from current encoder
  configuration and first-frame metadata to typed sequence/frame/tile-group
  header intent. It rejects unsupported formats and mismatches, is not
  re-exported, owns no writer, and does not produce packets.
- `ENC-SPEED-PRESETS` defines the typed runtime preset boundary used by
  `splot encode --speed`. The preset is stored in `EncoderRuntimeConfig`, not
  `EncoderConfig`, and currently does not affect packet output because no packet
  output path exists.
- `ENC-RESIDUAL-FOUNDATION` adds the first private encoder arithmetic primitive:
  checked source-minus-prediction residual blocks for the current borrowed 8-bit
  input surface. It is not re-exported, owns no writer, and does not produce
  packets.
- `ENC-FORWARD-TRANSFORM-FOUNDATION` adds a private 4x4 DCT_DCT DC-only
  forward-transform primitive for uniform residual blocks. It proves the no-op
  quant/dequant inverse handoff through `splot-recon`, but it is not a broad
  transform family, quantizer, token writer, or packet path.
- `ENC-QUANTIZATION-V0` adds a private fixed-quantizer stage for that first 4x4
  DCT_DCT DC-only coefficient subset. It validates qindex and dequant inputs,
  proves the `splot-recon` dequant/inverse handoff for qindex zero, and is not a
  quantization-matrix path, token writer, range encoder, rate-control mode, or
  packet path.
- `ENC-COEFFICIENT-TOKENIZATION-MINIMAL` adds a private token-fact bridge for the
  current luma 4x4 DCT_DCT DC-only top-left neutral-spatial-context quantized
  subset. It derives scan, EOB, begin-position, sign/magnitude, q-context, and
  ordered base-tier entropy-token records, including the low-frequency EOB base
  CDF row, and proves those records through the in-tree AV2 §8.2 symbol
  encoder/decoder. It is not broad coefficient syntax, neighbor-derived spatial
  contexts, coefficient base-range / `read_quant` extension, tile CDF lifecycle,
  tile-body emission, rate control, or a packet path.
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
  Annex B framing helpers, IVF helpers, the generic AV2 §8.2 `SymbolEncoder`
  primitive, and round-trip/fuzz coverage.
- This is still syntax/framing support, not an encoder. Coded tile payload
  generation is missing because broad encoder-owned §8.3 token/CDF selection,
  tile CDF lifecycle, coefficient-loop coverage, and the AV2 `decode_tile()`
  body path remain unimplemented.
- The private syntax IR and minimal header plan can stage future writer inputs,
  but no IR-to-writer serialization path exists yet.
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

As of the `encoder-coefficient-tokenization-minimal` branch point on
2026-06-18, PR #279 has merged into `main` as `8c9c5e27230e`, and this branch is
based on `origin/main` at that commit. PR #280
(`codex/decode-coeff-ordinary-branch-plane-tx-type`) is open against `main`; it
overlaps generated/tracking docs and coefficient-domain terminology, but its
code is decoder-local while this branch is private encoder tokenization. If
PR #280 lands first, rebase and regenerate matrix/status/coverage docs before
merge.

## Parked work

`toy-intra-encoder-v0` remains unchecked and parked. It is superseded as the
starting point for implementation by Baseline Encoder Profile v1. Future all-intra
work must be proposed with current writer, recon, validation, and conformance gates.
