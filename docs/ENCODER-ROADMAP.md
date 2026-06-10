# Encoder roadmap

`splot` is validator-first; the encoder comes later. This roadmap sequences the
**encoder-facing** work and ties each milestone to Feature IDs in
[`docs/IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml). Status is tracked
there, not here — see the generated [SPEC-COVERAGE.md](./SPEC-COVERAGE.md) /
[FEATURE-STATUS.md](./FEATURE-STATUS.md) for the live view.

## M0–M2 — Validator foundation *(see VALIDATOR-ROADMAP.md)*

The validator scaffold, header-level conformance, and high-level syntax
parsing milestones are validator scope: their sequencing, per-phase status,
and remaining gaps live in [VALIDATOR-ROADMAP.md](./VALIDATOR-ROADMAP.md)
(Phases 0–7), not here.

## M3 — Bitstream writer

A writer symmetric with the parsers for LEB128, OBU headers, trailing bits, byte
alignment, and the high-level syntax modeled so far.

- `ENC-BITSTREAM-WRITER`, `AV2-5.2.4-BYTE-ALIGNMENT`.

## M4 — Conformance harness *(see VALIDATOR-ROADMAP.md Phase 10 and CONFORMANCE.md)*

Public vectors, AVM differential testing, and snapshot `inspect` tests
(`CONF-PUBLIC-VECTORS`, `CONF-AVM-DIFF-HARNESS`, `CONF-INSPECT-SNAPSHOTS`) are
planned in [VALIDATOR-ROADMAP.md](./VALIDATOR-ROADMAP.md) Phase 10; the
encoder direction (`splot encode` → `avm decode`) activates once M3/M5 exist.

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
