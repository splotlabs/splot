## ADDED Requirements

### Requirement: Symbol encoder belongs to the bitstream writer foundation

`ENC-BITSTREAM-WRITER` SHALL include the generic AV2 § 8.2 symbol/range encoder
primitive as a required writer foundation before any future encoder change emits
real § 5.20 coded tile bodies. The matrix and generated writer/status docs SHALL
describe this primitive as the inverse of the existing `splot-core`
`SymbolDecoder` and SHALL keep its claim separate from § 8.3 CDF selection,
tile CDF lifecycle, syntax planning, coefficient tokenization, and coded tile
body generation.

#### Scenario: Matrix proof distinguishes primitive from tile syntax

- **WHEN** the symbol encoder primitive lands
- **THEN** `docs/IMPLEMENTATION-MATRIX.toml` and generated status docs SHALL
  record tests/fuzz evidence for the § 8.2 writer primitive under
  `ENC-BITSTREAM-WRITER`
- **AND** SHALL continue to mark coded tile body generation, coefficient syntax,
  § 8.3 CDF selection, and public encoder packet output as future or partial
  work unless those behaviors have separate runtime evidence.

#### Scenario: Public encoder behavior does not change

- **WHEN** only the symbol encoder primitive has landed
- **THEN** `splot encode` SHALL still fail honestly for lack of a coded-packet
  path
- **AND** no documentation SHALL claim Baseline Encoder Profile v1, minimal
  intra output, or broad AV2 encoder support from this primitive alone.
