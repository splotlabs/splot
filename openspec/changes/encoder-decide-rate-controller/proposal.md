## Why

The maintainer-approved minimal-working-encoder architecture takes every coding decision
behind a trait *seam* with a trivial fixed implementation now, so the optimization features
(RDO, rate control, search) land later as additive swaps rather than a rewrite. This adds the
first seam — `RateController` — establishing the `decide` module and the pattern.

## What Changes

- Add `ENC-DECIDE-RATE-CONTROLLER` as a private `splot-encode` encoder-tool feature.
- Add the `decide` module + the `RateController` trait + the `ConstantQp` implementation.
- The `Context` holds a `ConstantQp` (built from `EncoderConfig.qp`); `receive_packet`
  obtains `base_q_idx` via `frame_base_q_idx()` instead of reading `config.qp` directly.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: the quantizer decision is taken behind the `RateController` seam.

## Impact

- Affected code: `crates/splot-encode/src/{decide.rs (new), context.rs, lib.rs}` (+ tests).
- Output is byte-identical (`ConstantQp::frame_base_q_idx() == config.qp`); the cross-tool
  oracle is unchanged.
- Scope (explicitly NOT claimed): real rate control, the other four seams, per-block deltas.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status.
