## ADDED Requirements

### Requirement: Coefficient IDTX CDF row support status

The decoder support model SHALL track `DECODE-COEFF-IDTX-CDF-ROWS` as a
distinct crate-private row named `coeff-idtx-cdf-rows`. The row SHALL mark only
FSC/IDTX coefficient CDF row loading, selection, lifecycle handling, and mutable
symbol-reader handoff as partial support, and SHALL keep runtime `useFsc`
coefficient symbol sequencing, nonzero coefficient state writes, dequantization,
reconstruction, output, reference refresh, public APIs, and AVM/dav2d evidence
unsupported until separately implemented.

#### Scenario: Matrix records the IDTX CDF row boundary

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-idtx-cdf-rows` appears with Feature ID
  `DECODE-COEFF-IDTX-CDF-ROWS`
- **AND** it cites AV2 §5.20.7.27, §8.3.2, and §9.3 for syntax, CDF selection,
  and generated default row evidence
- **AND** it names focused CDF row tests
- **AND** broad runtime `coeffs()`, reconstruction, output, and reference
  support remain partial or unsupported in their existing rows
