## Why

The derived-base ordinary coefficient composer still accepts caller-supplied
sign inputs even though the sign-source branch is now derivable from the
first-pass `Level[]`, hidden-parity summary, plane, transform class, and DC
context lines. Composing that derivation into the derived-base pass is the next
narrow decoder-conformance brick before runtime `coeffs()` can stop fabricating
ordinary non-FSC sign facts.

Feature ID: `DECODE-COEFF-ORDINARY-DERIVED-SIGN-PASS`.

## What Changes

- Add a crate-private ordinary non-FSC coefficient-pass entry point or input
  shape that derives sign sources after the state-derived base/level first pass.
- Feed the derived `CoeffSignReadInput` records into the existing interleaved
  sign, `maxLevel`, `read_quant`, and signed `Quant[]` composition.
- Carry caller-resolved DC context-line and geometry facts only as the minimum
  state needed by the sign-source derivation; keep scan, transform facts,
  lossless, and nonzero runtime invocation staged.
- Update implementation/support matrices, decoder conformance coverage,
  roadmap notes, generated status docs, and OpenSpec artifacts.

## Capabilities

### New Capabilities

- `coeff-ordinary-derived-sign-pass`: ordinary non-FSC coefficient pass
  composition using derived base selectors and derived sign sources before the
  interleaved sign/quant stage.

### Modified Capabilities

- `decoder-support`: record the new partial decoder boundary and clarify that
  runtime `coeffs()` integration remains unsupported.

## Impact

Affected code is limited to crate-private `splot-decode` coefficient-loop
composition, tests, and tracking documents. There are no public API, dependency,
licensing, encoder, CLI, or fixture-output changes. The minimal runtime decode
path remains unchanged because real nonzero coefficient blocks still do not call
the ordinary coefficient-pass composer.
