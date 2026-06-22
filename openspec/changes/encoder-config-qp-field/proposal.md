## Why

Foundation brick 1 of the maintainer-directed minimal-working-encoder arc (minimal working
encoder → layered rav1e/SVT-AV1 optimization → AV2-specific). The quantizer lives as two
independent literals today (the skip frame's `base_q_idx` and the future quant qindex) — a
latent divergence hazard. Unify them under one `EncoderConfig.qp` field, establishing the
`RateController` seam's foundation (the minimal encoder is constant-QP).

## What Changes

- Add `ENC-CONFIG-QP-FIELD` as a private `splot-encode` encoder-tool feature.
- Add `EncoderConfig.qp: u8` (+ `DEFAULT_QP = 80`); thread it into the frame-header `base_q_idx`.
- `receive_packet` reads `self.config.qp`; restrict to the modeled q-context-0 range `1..=90`.
- Decode-verify at a non-default qp (40) in addition to the default (80).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: a single fixed-quantizer source threaded into the bitstream.

## Impact

- Affected code: `crates/splot-encode/src/{config.rs, context.rs, lib.rs,
  general_intra_trace/{mod,skip}.rs}` (+ the splot-cli oracle).
- Scope (explicitly NOT claimed): real residual coding, rate control, the full `get_qctx`
  q-context mapping (deferred, `TODO(spec: ENC-CONFIG-QP-FIELD)`).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status /
  spec coverage.
