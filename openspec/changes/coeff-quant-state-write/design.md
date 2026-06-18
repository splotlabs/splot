## Context

`DECODE-COEFF-SIGN-SYMBOL-READ` returns ordinary non-FSC sign summaries after
local `Level[]` state exists. The next §5.20.7.27 loop step calls
`read_quant`, applies hidden-parity and TCQ adjustments, clamps `culLevel`,
derives `dcCategory`, and writes signed `Quant[pos]`. The §5.20.7.28
`read_quant` syntax reader is still a separate piece of work because it
consumes variable-length literal syntax and updates `hrLevelAvg`.

## Goals / Non-Goals

**Goals:**

- Add `DECODE-COEFF-QUANT-STATE-WRITE` as a focused feature row and decoder
  support row.
- Add a crate-private `coeff_loop/quant_state.rs` helper that applies
  caller-provided `read_quant` outputs to local transform-block state.
- Validate all caller facts that can be checked before mutation: input counts,
  scan-entry identity, sign-entry identity, local level coordinates, and flat
  `Quant[pos]` bounds.
- Return per-entry summaries plus final `culLevel`, `dcCategory`, `tcqState`,
  and `hrLevelAvg` facts for later context-line updates and parser composition.
- Prove the boundary with self-contained unit tests.

**Non-Goals:**

- No runtime `decode_block()` / `coeffs()` integration and no decode-output
  change.
- No implementation of §5.20.7.28 `read_quant` bit parsing, `q_length_bit`,
  `golomb_length_bit`, `coeff_rem`, or `hrLevelAvg` derivation from symbols.
- No `QuantSign[]` writes; the IDTX sign-state path is separate.
- No derivation of `isLf`, `maxLevel`, parity hiding, TCQ enablement,
  `Lossless`, or scan tables from real block syntax.
- No dequantization, inverse transform, residual add, reconstruction, reference
  refresh, public API, AVM/dav2d invocation, scheduler change, or dependency
  change.

## Decisions

1. Keep `read_quant` parsing caller-owned for this brick.

   Rationale: §5.20.7.28 has its own literal loop, Golomb-style extension, and
   `hrLevelAvg` update. Accepting a caller-provided quant result lets this
   change land the state mutation boundary without coupling it to variable
   literal parsing.

2. Preflight all count, identity, and coordinate checks before mutation.

   Rationale: `Quant[]` writes are local state changes. The helper can make
   those transactional for caller-fact mismatches by validating the whole input
   list first, then applying writes in scan order.

3. Model ordinary non-FSC TCQ enablement as a caller-selected fact, with the
   quant-write state reset held by the helper.

   Rationale: §5.20.7.27 resets `tcqState` to `0` immediately before the
   quant-write loop. This helper can apply the post-`read_quant` formula from
   caller-supplied `useTcq` and `lossless` facts while owning that reset until
   runtime `coeffs()` wiring derives the surrounding block facts from syntax.

4. Leave `QuantSign[]` untouched.

   Rationale: §5.20.7.27 writes `QuantSign[pos]` in the IDTX loop, while the
   ordinary non-FSC branch targeted here writes signed `Quant[pos]` only. This
   avoids inventing a sign-state write that the targeted branch does not model.

## Risks / Trade-offs

- Caller-owned quant facts can be inconsistent with the future §5.20.7.28 reader
  -> mitigate with explicit input records, scan/sign identity checks, and matrix
  notes that quant syntax parsing is out of scope.
- TCQ state is data-dependent and easy to overclaim -> keep it crate-private,
  start the quant-write pass from the spec reset state, and cover it with
  focused tests rather than wiring runtime block syntax.
- This still does not change runtime output -> keep docs and matrices explicit
  that runtime `coeffs()`, `read_quant`, and reconstruction remain deferred.
