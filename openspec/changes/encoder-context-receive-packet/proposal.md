## Why

**Milestone A keystone.** The public `splot-encode` push/pull `Context` has had a deterministic
lifecycle but `receive_packet` could never return a real packet — the encoder produced no output
through its public API. This wires `receive_packet` to emit a real, decodable AV2 packet from a
sent input frame, for the input subset the minimal encoder can encode **losslessly**, proving the
end-to-end public encoder path (`send_frame` → `receive_packet` → a packet that decodes back to
the input).

## What Changes

- `Context` retains input **pixels** (a private `QueuedFrame` owning the visible Y/U/V samples)
  instead of only `FrameInfo`, so `receive_packet` can inspect the input after the borrowed
  `Frame` ends.
- `receive_packet` produces a real `Packet` when the queued frame is the supported subset — a
  64x64 frame whose every visible sample is the 128 no-neighbour DC predictor — by emitting the
  decode-proven `emit_minimal_intra_skip_ivf()`. Such a frame has zero residual, so the skip
  frame's flat-128 reconstruction equals the input (the honesty invariant: output decodes to
  input, never a canned frame).
- Any other frame is retired without a packet (no wrong/canned output). A typed error for
  unsupported input + broader input handling are follow-ups.
- Add the cross-crate e2e oracle: build an all-128 frame, drive the public `Context`
  (`send_frame`/`flush`/`receive_packet`), decode the packet, assert `decode(encode(input)) ==
  input`.
- Update the stale `splot-encode` / `Context` module docs that claimed no packet production.

## Capabilities

### Modified Capabilities

- `encoder-tools`: add the first real public-API packet production (Milestone A keystone).

## Impact

- Affected code: `crates/splot-encode/src/context.rs`, `crates/splot-encode/src/lib.rs`,
  `crates/splot-cli/tests/encode_decode_roundtrip.rs`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated status/coverage.
- Public API impact: `Context::receive_packet` now returns a `Packet` for the supported subset
  (previously always `NeedMoreData`/`Finished`). No type/signature change. No dependency-graph
  change.
