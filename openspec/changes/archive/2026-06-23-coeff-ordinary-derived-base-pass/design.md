## Context

`DECODE-COEFF-ORDINARY-PASS-COMPOSE` currently composes the ordinary non-FSC
coefficient path from nonzero EOB through signed `Quant[]` writes, but it still
requires caller-provided base/base-range selector inputs and caller-provided
hidden-parity and `sumAbs1` facts. `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS`
already implements the AV2 §5.20.7.27 first pass: it walks checked scan entries,
derives base selectors from evolving `Level[]`, updates first-pass `tcqState`,
`sumAbs1`, `numNz`, and `isHidden`, and writes local `Level[]`.

This change composes those two loaded-but-unwired boundaries. It does not make
runtime `coeffs()` call the ordinary pass yet; callers still provide scan,
geometry, plane, transform-class, parity, TCQ, lossless, and sign-source facts.

## Goals / Non-Goals

**Goals:**

- Add a crate-private ordinary-pass entry point that derives base selectors and
  local `Level[]` state through the existing first-pass helper.
- Feed the first-pass `isHidden` and `sumAbs1` into the existing interleaved
  sign/`read_quant`/signed `Quant[]` composition.
- Derive the second-pass `maxLevel` plane and transform-class facts from the
  same first-pass config, avoiding duplicate caller facts.
- Preserve existing explicit-base tests and add focused derived-base tests for
  equivalence, hidden parity, and pre-consumption failures.
- Keep tracking docs and generated support/coverage status honest.

**Non-Goals:**

- No runtime `coeffs()` integration, real scan-table derivation from `txSz`,
  sign-source derivation from `Level[]`/transform class, tile context commits,
  dequantization, inverse transform, residual add, reconstruction, reference
  refresh, public API, CLI, encoder, dependency, or fixture-output changes.
- No new AV2 constants, tables, CDF contents, or copied third-party code.

## Decisions

1. **Add a sibling derived-base entry point.**
   The existing `apply_nonzero_coeff_ordinary_pass` remains available for staged
   tests and explicit selector boundaries. A new function composes the derived
   first pass without changing the older function's caller contract.

2. **Reuse the first-pass result as the source of truth.**
   The derived-base composer stores the `NonZeroCoeffBaseDerivedLevelPass`
   result and clones its local block for the second pass. Tests can then inspect
   derived selector inputs, base reads, and the first-pass summary without
   duplicating derivation logic in the ordinary composer.

3. **Remove duplicate hidden/sumAbs1 caller facts from the new input.**
   The derived-base composer builds `CoeffQuantPassConfig` with
   `is_hidden = first_pass.is_hidden()`, `sum_abs1 = first_pass.sum_abs1()`,
   `use_tcq = base_config.use_tcq`, `lossless = input.lossless`, and
   `hr_level_avg = 0` at block entry, matching the §5.20.7.27 sequence.

4. **Keep sign sources caller-resolved for now.**
   Sign selection depends on post-level state, plane, transform class, and DC
   contexts. This change deliberately stops before deriving sign inputs so the
   first-pass composition remains small and independently reviewable.

## Risks / Trade-offs

- **Overclaiming runtime support** -> Matrix, roadmap, and OpenSpec scenarios
  must state that runtime `coeffs()` remains unwired and decode output remains
  unchanged.
- **Divergent old/new composer behavior** -> Add an equivalence test where
  derived-base composition matches the explicit-base composition when supplied
  with the first pass's derived inputs and summary.
- **Hidden parity wiring mistakes** -> Add a derived-base test where the first
  pass activates hidden parity and the second pass consumes hidden DC behavior
  from the first-pass summary, not caller-supplied quant facts.
- **Mutation before preflight failure** -> Preserve existing fail-before-read
  behavior for scan/base preflight and add a derived-base failure check for
  malformed first-pass config before any symbol/CDF mutation.
