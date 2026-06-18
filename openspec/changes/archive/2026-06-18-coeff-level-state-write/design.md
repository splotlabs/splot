## Context

`DECODE-COEFF-BASE-SYMBOL-READ` returns decoded ordinary non-FSC coefficient
levels for checked scan-walk entries. The next normative operation in
§5.20.7.27 is `Level[row][col] = level`; the later sign pass then reads
`Level[]` before `read_quant` writes `Quant[]` and `QuantSign[]`.

The existing `TransformCoeffBlockState` already owns row-major `Level[]`,
`QuantSign[]`, and `Quant[]` arrays and exposes checked setters. This change
should compose those existing pieces rather than widening the coefficient loop
to sign or quantization semantics.

## Goals / Non-Goals

**Goals:**

- Add `DECODE-COEFF-LEVEL-STATE-WRITE` as a focused feature row and
  decoder-support row.
- Add a new `coeff_loop/level_state.rs` helper that consumes a
  `NonZeroCoeffBlockStart`, validates a checked scan walk against decoded
  `CoeffBaseSymbolRead` records, and writes each returned level to
  `Level[row][col]`.
- Validate every read/walk pairing and preflight block coordinates before
  mutating the local transform-block state.
- Keep `QuantSign[]` and `Quant[]` zeroed and untouched.
- Prove the boundary with self-contained unit tests.

**Non-Goals:**

- No runtime `decode_block()` / `coeffs()` integration and no decode-output
  change.
- No derivation of `get_scan`, `compute_tx_type`, `get_lf_limits`,
  `baseLevels`, `tcqState`, parity-hiding, or sign contexts from real block
  syntax.
- No sign reads, `dc_sign`, `idtx_sign`, `QuantSign[]` writes, `Quant[]`
  writes, `read_quant`, dequantization, inverse transform, residual add,
  reconstruction, reference refresh, public API, AVM/dav2d invocation,
  scheduler change, or dependency change.

## Decisions

1. Consume the nonzero block start and return the updated block state.

   Rationale: this mirrors the staged `coeffs()` flow: EOB allocation/read
   produces a local block, scan/base reads produce levels, and this boundary
   returns the next local-block state for later sign and quantization passes.

   Alternative considered: mutate a borrowed block in place. That would make
   transactionality harder to prove for preflight errors and would expose a
   partially-written state if a future caller accidentally paired mismatched
   facts.

2. Validate before writing.

   Rationale: scan-entry count and identity errors are caller bugs. Checking all
   of them, plus every target `Level[row][col]` coordinate, before the write
   loop keeps the helper fail-closed and easy to test.

   Alternative considered: rely on `set_level` during the write loop. That would
   still be bounds-safe, but an error could leave earlier entries written.

3. Keep level state separate from sign and quant state.

   Rationale: AV2 §5.20.7.27 has a clean boundary between the reverse-order
   base/base-range level pass and the later sign/`read_quant` pass. A separate
   helper keeps this PR reviewable and avoids claiming decode-output progress
   before `Quant[]` exists.

## Risks / Trade-offs

- A caller can pass a scan walk from a different block start -> mitigate by
  preflighting the returned row/column facts against the consumed block before
  any writes.
- This helper still does not change runtime output -> keep matrices and roadmap
  explicit that the minimal runtime path remains all-zero only.
- The helper clones no large global state, but it consumes the local block state;
  later runtime integration may need small API reshaping once signs and
  `read_quant` are added.
