## Why

The ordinary non-FSC coefficient composer now derives base selectors, sign
sources, `read_quant`, and signed `Quant[]`, but its nonzero result still stops
before the §5.20.7.27 end-of-`coeffs()` context-line writes. Committing
`culLevel` and `dcCategory` back into tile coefficient context state is the next
narrow decoder-conformance brick before runtime `coeffs()` can use the composer
without fabricating above/left context state.

Feature ID: `DECODE-COEFF-NONZERO-CONTEXT-COMMIT`.

## What Changes

- Add a crate-private nonzero ordinary coefficient-pass context-commit boundary
  that updates `AboveLevelContext`, `LeftLevelContext`, `AboveDcContext`, and
  `LeftDcContext` after the derived-base/derived-sign ordinary pass succeeds.
- Source `culLevel` and `dcCategory` from the final quant-state summary already
  produced by the ordinary pass, and source plane/geometry facts from a compact
  caller-resolved context-update config.
- Preserve transactional behavior: pass failures must not mutate tile context
  lines, and invalid context-update geometry must fail after the pass while
  preserving the pre-existing context state.
- Update implementation/support matrices, decoder conformance coverage,
  roadmap notes, generated status docs, and OpenSpec artifacts.

## Capabilities

### New Capabilities

- `coeff-nonzero-context-commit`: ordinary non-FSC nonzero coefficient pass
  composition that commits final `culLevel` and `dcCategory` to tile coefficient
  context lines after the pass succeeds.

### Modified Capabilities

- `decoder-support`: record the new partial decoder boundary and clarify that
  runtime `coeffs()` integration remains unsupported.

## Impact

Affected code is limited to crate-private `splot-decode` coefficient-loop
composition, tests, and tracking documents. There are no public API, dependency,
licensing, encoder, CLI, or fixture-output changes. The minimal runtime decode
path remains unchanged because real nonzero coefficient blocks still do not call
the ordinary coefficient-pass composer.
