## Context

`DECODE-COEFF-SIGN-SYMBOL-READ` reads caller-selected sign sources after local
`Level[]` state exists. `DECODE-COEFF-ORDINARY-DERIVED-BASE-PASS` now supplies
that local `Level[]` and first-pass hidden-parity summary, but the ordinary pass
still receives fabricated `CoeffSignReadInput` records from tests/callers.

The AV2 §5.20.7.27 sign-source branch is now small enough to isolate: for each
checked scan entry, read a sign when `Level[row][col] != 0` or when hidden
parity applies to `c == 0` and `sumAbs1 > 0`; choose luma DC `dc_sign`, luma
axis `dc_sign_horz_vert`, or raw `sign_bit` from the entry coordinates,
transform class, and plane. The §8.3.2 `dc_sign` CDF context is already
implemented as `dc_sign_ctx`.

## Goals / Non-Goals

**Goals:**

- Add a crate-private helper that derives `CoeffSignReadInput` records for a
  checked ordinary non-FSC scan walk.
- Use local `TransformCoeffBlockState::level_at` as the source for the
  post-first-pass `Level[]` branch.
- Use `dc_sign_ctx` with caller-provided above/left DC context slices and
  block coordinates for luma DC `dc_sign`.
- Cover luma DC, horizontal-axis, vertical-axis, generic `sign_bit`, skipped
  zero-level entries, hidden-parity zero-level DC, and state-error behavior.
- Update tracking docs and generated support/coverage status honestly.

**Non-Goals:**

- No runtime `coeffs()` integration, no changes to existing ordinary-pass
  inputs, no `QuantSign[]` writes, no tile context commits, no scan/transform
  derivation from real syntax, no dequantization, no reconstruction, no output
  changes, no public API, no encoder work, and no dependency or crate graph
  changes.
- No new AV2 constants, tables, CDF contents, or copied third-party code.

## Decisions

1. **Add derivation beside the sign reader.**
   `sign_symbol.rs` already owns `CoeffSignReadInput`,
   `CoeffSignReadSource`, and `CoeffDcSignSelector`. Keeping the derivation
   there avoids a new module and keeps the source file under the soft line
   budget.

2. **Use a compact config struct for caller-resolved block facts.**
   The helper takes `CoeffSignSourceDeriveConfig` containing the CDF q-context,
   plane, plane type, transform class, hidden flag, `sumAbs1`, DC context
   slices, and block x/y/w/h in 4x4 units. This mirrors the remaining caller
   facts and avoids importing reconstruction state or broader syntax models.

3. **Return inputs, not reads.**
   The helper does not consume symbols or mutate CDFs. It produces
   `CoeffSignReadInput` records that the existing sign reader can validate and
   consume later. A follow-on brick can wire these records into the derived-base
   ordinary composer without mixing derivation and symbol-consumption changes.

4. **Keep hidden parity explicit.**
   The branch reads a sign for `isHidden && c == 0 && sumAbs1 > 0` even when
   the local level is zero. This matches the existing quant-pass hidden-parity
   preflight and prevents the derivation from silently producing `None` for the
   final parity carrier.

## Risks / Trade-offs

- **Risk: Overclaiming runtime decode support** -> Matrix, roadmap, and
  OpenSpec rows must state that runtime `coeffs()` still does not call the
  derived source helper and decode output is unchanged.
- **Risk: Plane/type confusion for sign CDF selectors** -> Tests cover luma DC
  and luma transform-axis CDF selectors separately from chroma/generic
  `sign_bit`, with expected `isHidden` group and DC context values.
- **Risk: Hidden-parity edge cases diverge from sign-reader preflight** -> Tests
  cover zero-level hidden final DC with positive `sumAbs1`, plus zero-level
  non-hidden skip behavior.
- **Risk: State coordinate errors appear after partial output allocation** ->
  The helper only allocates an output vector and returns a typed state error;
  it consumes no symbol/CDF state, so failure is transactionally harmless.
