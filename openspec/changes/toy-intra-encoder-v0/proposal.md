# Change: toy-intra-encoder-v0

## Feature IDs

- `ENC-INTRA-TOY-V0`

## Why

A minimal, deliberately simple intra-only path proves the writer + headers +
validation are sufficient to produce a *legal* AV2 stream end to end, before any
real encoder work.

## Scope

- Crates/modules: `splot-encode` (`context`), driving `splot-core` writer.
- Depends on: `ENC-BITSTREAM-WRITER`, `AV2-5.4-SEQUENCE-HEADER`, and enough
  frame/tile syntax to emit one legal frame.

## Non-goals

- No prediction/transform/quantization quality work.
- No rate control or speed presets.
- No invented syntax to make a stream "work".

## Acceptance criteria

- [ ] The toy path emits a single intra frame using only writer-supported syntax.
- [ ] The output validates clean under `splot validate`.
- [ ] (Stretch) AVM decodes the output (`CONF-AVM-DIFF-HARNESS`).
- [ ] Matrix row and proof are updated.

> Status: **proposed**. Blocked on the writer and header work above.
