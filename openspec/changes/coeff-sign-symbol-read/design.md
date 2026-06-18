## Context

`DECODE-COEFF-LEVEL-STATE-WRITE` applies decoded ordinary non-FSC levels to the
local transform block. In AV2 §5.20.7.27 the next pass reads signs for entries
whose `Level[row][col]` is nonzero, plus the parity-hidden exception when
`isHidden && c == 0 && sumAbs1 > 0`. Depending on caller-resolved facts, the sign
source is `dc_sign`, `dc_sign_horz_vert`, or a raw `sign_bit` literal. The
`QuantSign[]` write happens later, after `read_quant`, so it should stay out of
this change.

## Goals / Non-Goals

**Goals:**

- Add `DECODE-COEFF-SIGN-SYMBOL-READ` as a focused feature row and
  decoder-support row.
- Add a new `coeff_loop/sign_symbol.rs` helper that reads caller-selected sign
  sources over checked scan-walk entries and local `Level[]` state.
- Validate all caller facts that can be checked without consuming syntax before
  the first read: input count, scan-entry identity, local level coordinates, and
  missing sign sources for nonzero levels.
- Return per-entry sign summaries for later quantization.
- Prove the boundary with self-contained unit tests.

**Non-Goals:**

- No runtime `decode_block()` / `coeffs()` integration and no decode-output
  change.
- No derivation of `dc_sign` vs `dc_sign_horz_vert` vs `sign_bit` policy from
  real `txClass`, plane, row/column, parity hiding, or DC-context state.
- No `QuantSign[]` writes, `Quant[]` writes, `read_quant`, dequantization,
  inverse transform, residual add, reconstruction, reference refresh, public
  API, AVM/dav2d invocation, scheduler change, or dependency change.

## Decisions

1. Keep sign-source selection caller-owned.

   Rationale: the source branch depends on transform class, plane, DC position,
   and parity-hidden state that are still caller-resolved in the staged
   coefficient loop. The helper should sequence reads and preserve checked entry
   identity without coupling this PR to broader transform/block syntax.

2. Enforce signs for nonzero levels, but allow caller-forced reads for zero
   levels.

   Rationale: nonzero `Level[]` entries always read a sign in the ordinary path.
   The parity-hidden exception can also read a sign at `c == 0` even when the
   level is zero, so callers must be able to request a CDF or literal read for a
   zero-level entry.

3. Do not write `QuantSign[]`.

   Rationale: the spec writes `QuantSign[pos]` after `read_quant` and after
   signed `Quant[pos]` production. Returning sign summaries now keeps the next
   `read_quant` brick clean and avoids out-of-order state claims.

## Risks / Trade-offs

- Caller-owned sign source selection can be mismatched with future transform
  facts -> mitigate with explicit input records, exact scan-entry checks, and
  matrix notes that policy derivation is out of scope.
- A later invalid CDF selector can fail after earlier signs were read -> this
  matches the spec read order. Tests cover the no-consumption behavior for an
  invalid selector reached on the first entry.
- This still does not change runtime output -> keep all docs and matrices
  explicit that `QuantSign[]`, `Quant[]`, and reconstruction remain deferred.
