## ADDED Requirements

### Requirement: Decoder support tracks FSC quant pass
The decoder support model SHALL track `DECODE-COEFF-FSC-QUANT-PASS` as a
distinct partial decoder-support row.

#### Scenario: FSC quant support row exists
- **WHEN** decoder support status is generated
- **THEN** it includes a partial row for `DECODE-COEFF-FSC-QUANT-PASS` describing
  the loaded-but-unwired FSC interleaved sign/`read_quant` and signed `Quant[]`
  pass, its local tests, and the remaining runtime `coeffs()`, context commit,
  dequantization, reconstruction, output, reference, inter-prediction, and filter
  gaps

### Requirement: Decoder conformance coverage maps FSC quant pass
The decoder conformance coverage model SHALL map `DECODE-COEFF-FSC-QUANT-PASS`
to the decode-relevant coefficient syntax and symbol/CDF coverage groups.

#### Scenario: FSC quant coverage appears in generated docs
- **WHEN** decoder conformance coverage is generated
- **THEN** the `tile-group-and-payload-syntax` and `symbol-and-cdf-process`
  coverage groups reference `DECODE-COEFF-FSC-QUANT-PASS`
