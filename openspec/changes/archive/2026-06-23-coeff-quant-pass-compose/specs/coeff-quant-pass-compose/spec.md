## ADDED Requirements

### Requirement: Ordinary non-FSC quant pass composition
The decoder SHALL provide a crate-private ordinary non-FSC coefficient helper
that composes AV2 § 5.20.7.28 `read_quant` parsing with AV2 § 5.20.7.27 local
`Quant[]` state writes after checked scan entries, local `Level[]` values, and
sign summaries are available.

#### Scenario: Read-quant values feed quant writes
- **GIVEN** checked scan entries, local levels, sign summaries, caller-derived
  `maxLevel` values, and block-level hidden, TCQ, and lossless facts
- **WHEN** the composed helper runs on a bit payload that reaches `read_quant`
  literal syntax
- **THEN** it consumes only the reached `read_quant` literal bits
- **AND** it applies the decoded `quant` records through the existing
  quant-state writer
- **AND** it returns both the raw `read_quant` records and the final
  quant-state summary

#### Scenario: Bad caller facts fail before literal consumption
- **GIVEN** malformed caller facts such as mismatched scan entries, inconsistent
  sign levels, invalid `maxLevel - useTcq`, missing hidden-parity sign syntax,
  hidden parity paired with TCQ or lossless facts, or out-of-range `Quant[]`
  positions
- **WHEN** the composed helper is called
- **THEN** it returns a typed crate-private error before consuming any
  `read_quant` literal bits
- **AND** the local `TransformCoeffBlockState` remains unchanged

#### Scenario: Runtime decode remains unchanged
- **WHEN** the existing minimal decode runtime runs
- **THEN** it does not call the composed helper yet
- **AND** existing fixture output and unsupported-feature diagnostics remain
  unchanged until a later runtime `coeffs()` integration change wires real block
  facts into the helper
