## Why

The ordinary coefficient-pass composer still accepts caller-supplied
base/base-range selectors even though the state-derived first pass now derives
those selectors from evolving `Level[]`, TCQ, and hidden-parity state. Composing
that first pass into the ordinary pass is the next narrow decoder-conformance
brick before runtime `coeffs()` can consume real nonzero coefficient blocks.

Feature ID: `DECODE-COEFF-ORDINARY-DERIVED-BASE-PASS`.

## What Changes

- Add a crate-private ordinary non-FSC coefficient-pass entry point that derives
  base/base-range selectors and writes first-pass `Level[]` state through
  `apply_nonzero_coeff_base_derived_level_pass`.
- Carry the derived first-pass `isHidden` and `sumAbs1` facts into the existing
  interleaved sign, `maxLevel`, `read_quant`, and signed `Quant[]` composition.
- Keep the existing caller-supplied-base ordinary pass for tests and staged
  boundaries while adding focused coverage for the derived-base composer.
- Update implementation/support matrices, decoder conformance coverage,
  roadmap notes, generated status docs, and OpenSpec artifacts.

## Capabilities

### New Capabilities

- `coeff-ordinary-derived-base-pass`: ordinary non-FSC coefficient pass
  composition using derived base selectors, first-pass `Level[]`, hidden-parity,
  and `sumAbs1` facts.

### Modified Capabilities

- `decoder-support`: record the new partial decoder boundary and clarify that
  runtime `coeffs()` integration remains unsupported.

## Impact

Affected code is limited to crate-private `splot-decode` coefficient loop
composition, tests, and tracking documents. There are no public API, dependency,
licensing, encoder, CLI, or fixture-output changes. The minimal runtime decode
path remains unchanged because real nonzero coefficient blocks still do not call
the ordinary coefficient-pass composer.
