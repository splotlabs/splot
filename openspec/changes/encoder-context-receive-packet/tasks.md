## 1. Context wiring

- [x] 1.1 Retain input pixels in a private `QueuedFrame` (owned visible Y/U/V); change `input_queue` to hold it; copy samples in `send_frame`.
- [x] 1.2 `receive_packet` emits the skip packet for a 64x64 all-128 frame; retires any other frame without a packet; emitter failure is terminal (`Failed`).

## 2. Oracle + docs

- [x] 2.1 Context unit tests: supported (64x64 all-128 → skip packet → Finished) + unsupported (non-flat 64x64 → no packet); all existing lifecycle tests still pass.
- [x] 2.2 Cross-crate e2e oracle: drive the public `Context`, decode the packet, assert `decode(encode(all-128)) == all-128`.
- [x] 2.3 Update the stale `splot-encode`/`Context` "no packet production" module docs.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-CONTEXT-RECEIVE-PACKET` to the implementation matrix and refresh generated docs.
- [x] 3.2 Keep tracking honest: the first real public-API packet, lossless ONLY for the all-128 subset; forward quantization / non-uniform input / mode-decision / RDO are §10 maintainer-gated follow-ups.
- [x] 3.3 Run OpenSpec validation, focused encode/cli tests, feature-status checks, and `cargo xtask ci`.
