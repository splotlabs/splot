# Encoder roadmap

`splot` is validator-first; the encoder comes later. This roadmap sequences the
work and ties each milestone to Feature IDs in
[`docs/IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml). Status is tracked
there, not here — run `cargo xtask feature-status` for the live view.

## M0 — Repo + validator scaffold *(done)*

Annex B, LEB128, AV2 OBU header, structured diagnostics, the `inspect` CLI, and
no-panic fuzz/property coverage.

- `AV2-4.11.6-LEB128`, `AV2-5.2.2-OBU-HEADER`, `AV2-5.2.1-OBU-TYPE`,
  `AV2-B-ANNEXB-OBU-ENVELOPE`, `CLI-VALIDATE`, `CLI-INSPECT`, `CONF-FUZZ-NO-PANIC`.

## M1 — Header-level conformance

OBU type table, layer-id rules, trailing bits, reserved OBU handling.

- `AV2-5.2.2-OBU-HEADER` (validate), `AV2-5.2.3-TRAILING-BITS`,
  `AV2-5.3-RESERVED-OBU`, `AV2-7.3-OBU-ORDERING`.

## M2 — High-level syntax parsing

Sequence header, layer configuration record, operating point set, frame-header
skeleton.

- `AV2-5.4-SEQUENCE-HEADER`, `AV2-5.8-LAYER-CONFIG-RECORD`,
  `AV2-5.10-OPERATING-POINT-SET`, `AV2-5.18-FRAME-HEADER`.

## M3 — Bitstream writer

A writer symmetric with the parsers for LEB128, OBU headers, trailing bits, byte
alignment, and the high-level syntax modeled so far.

- `ENC-BITSTREAM-WRITER`, `AV2-5.2.4-BYTE-ALIGNMENT`.

## M4 — Conformance harness

Public vectors (when available), AVM differential testing, snapshot `inspect` tests.

- `CONF-PUBLIC-VECTORS`, `CONF-AVM-DIFF-HARNESS`, `CONF-INSPECT-SNAPSHOTS`.

## M5 — Minimal experimental encoder path

Produce deliberately simple streams using only implemented, writer-validated syntax.

- `ENC-INTRA-TOY-V0` (with `AV2-5.19-TILE-GROUP` as needed).

## M6 — Real encoder experiments

Prediction, transforms, quantization, rate control, speed presets, threading.

- `ENC-RATE-CONTROL-V0`, `ENC-SPEED-PRESETS` (plus future rows for prediction,
  transform, quantization, and threading as they are scoped).

## M7 — Performance and productization

Benchmarks, a SIMD policy (behind narrowly-scoped, documented, tested modules),
corpus regression, CLI polish.

- The `perf` stage across the rows above, plus `ENC-SPEED-PRESETS`. SIMD work is
  gated by the workspace `unsafe_code = "forbid"` rule (see
  [ARCHITECTURE.md](./ARCHITECTURE.md)).
