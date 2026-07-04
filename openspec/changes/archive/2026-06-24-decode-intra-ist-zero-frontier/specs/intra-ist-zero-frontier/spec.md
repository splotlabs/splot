## ADDED Requirements

### Requirement: local decoder mission Intra IST Zero Frontier

The decoder SHALL track `DECODE-INTRA-IST-ZERO-FRONTIER` as a partial
runtime prerequisite for the local decoder mission decode mission. For the covered intra luma
DCT_DCT residual subset, when AV2 §5.20.7.29 requires secondary-transform syntax
(`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-29`), the decoder SHALL
read `sec_tx_type` through the AV2 §8.3.2 tile CDF row
(`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`) and SHALL continue only
when the symbol selects no secondary transform.

#### Scenario: Zero secondary transform remains synchronized

- **WHEN** the local decoder mission residual path reaches an intra luma DCT_DCT block
  whose §5.20.7.29 conditions require `sec_tx_type`
- **AND** the decoded `sec_tx_type` symbol is zero
- **THEN** the decoder continues through the existing DCT-only coefficient path
- **AND** it does not emit `unsupported_dctonly_residual_intra_ist` for that
  zero-secondary-transform block

#### Scenario: Active secondary transform stays fail-closed

- **WHEN** an intra luma DCT_DCT residual block decodes a non-zero
  `sec_tx_type`
- **THEN** the decoder consumes the intra `most_probable_stx_set` symbol in AV2
  §5.20.7.29 order
- **AND** it emits a structured unsupported-feature diagnostic before claiming
  coefficient or reconstruction support for active secondary transforms

#### Scenario: No successful local decoder mission decode claim

- **WHEN** the intra IST zero frontier has been implemented
- **THEN** `DECODE-INTRA-IST-ZERO-FRONTIER` remains partial
- **AND** the decoder does not claim successful local decoder mission output, raw/Y4M output,
  reference refresh, AVM/dav2d byte equality, or broad secondary-transform
  support
