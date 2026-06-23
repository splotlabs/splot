## ADDED Requirements

### Requirement: Decode coefficient quantized-state writes
The decoder coefficient-loop boundary SHALL provide a crate-private ordinary
non-FSC quantized-state helper tracked by `DECODE-COEFF-QUANT-STATE-WRITE`.
The helper SHALL accept local `Level[]` state, a checked scan walk, sign-read
summaries, and caller-provided `read_quant` outputs, SHALL apply the
§5.20.7.27 quantized-coefficient state effects in scan-walk order, and SHALL
write local `Quant[pos]` state without mutating `QuantSign[]` and without
running dequantization, inverse transform, residual add, reconstruction, or
runtime `coeffs()`.

#### Scenario: Quantized coefficients are written in scan-walk order
- **WHEN** the helper receives matching sign and quant inputs for a checked scan
  walk
- **THEN** it applies hidden-parity, optional TCQ, and sign adjustments to each
  caller-provided quant value in scan order
- **AND** it writes the signed value to `Quant[pos]`
- **AND** `QuantSign[]` remains unchanged

#### Scenario: Caller fact mismatches are transactional
- **WHEN** the quant inputs do not match the checked scan walk, sign summaries,
  or local transform-block coordinates
- **THEN** the helper returns a typed error before mutating `Quant[]`,
  `culLevel`, `dcCategory`, or TCQ state

#### Scenario: Runtime quant syntax remains out of scope
- **WHEN** the minimal runtime decode path is exercised after this change
- **THEN** it still does not execute nonzero `read_quant` syntax
- **AND** it does not dequantize, transform, add residuals, reconstruct pixels,
  or change fixture output
