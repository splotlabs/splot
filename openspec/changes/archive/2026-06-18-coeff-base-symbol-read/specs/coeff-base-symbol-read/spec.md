## ADDED Requirements

### Requirement: Decode coefficient base symbol reads
The decoder coefficient-loop boundary SHALL provide a crate-private ordinary
non-FSC, non-IDTX coefficient base symbol-read helper tracked by
`DECODE-COEFF-BASE-SYMBOL-READ`. The helper SHALL accept checked scan-walk
entries plus caller-resolved CDF selectors for `coeff_base_eob`, `coeff_base`,
and conditionally `coeff_br`, SHALL consume those rows through
`SymbolDecoder::read_symbol`, and SHALL return decoded base and base-range
symbol values without writing coefficient state or claiming runtime decode
support.

#### Scenario: Base symbols are read in scan-walk order
- **WHEN** the helper receives a `NonZeroCoeffScanWalk` and matching per-entry
  read inputs
- **THEN** it reads exactly one base symbol for each scan entry in walk order
- **AND** the first visited entry may use a `coeff_base_eob` selector whose
  decoded level is biased by one, while later entries may use `coeff_base`
  selectors without that bias
- **AND** the returned records preserve the checked scan index, raster
  position, row, and column facts

#### Scenario: Base-range read is conditional
- **WHEN** a decoded base level is greater than the caller-provided base-level
  threshold and the entry enables base-range reading
- **THEN** the helper reads the caller-provided `coeff_br` row and adds the
  decoded base-range symbol to the returned level
- **AND** entries whose base level does not cross the threshold, or whose
  base-range read is disabled, do not touch a `coeff_br` row

#### Scenario: Invalid selectors fail transactionally
- **WHEN** an entry names an out-of-range coefficient CDF selector that is
  reached by the spec read order
- **THEN** the helper returns a typed selector error before consuming symbol bits
  or mutating CDF rows for that failed read
- **AND** invalid selectors for unreached conditional base-range reads do not
  affect the already-decoded base symbol

#### Scenario: CDF update mode is honored
- **WHEN** the helper reads selected rows with CDF updates enabled
- **THEN** only the rows reached by the read sequence are updated
- **AND** when CDF updates are disabled, the same reads advance the symbol
  decoder but leave selected CDF rows unchanged

#### Scenario: Runtime coefficient decode remains out of scope
- **WHEN** the minimal runtime decode path is exercised after this change
- **THEN** it still does not execute nonzero coefficient base symbol reads
- **AND** it does not write nonzero `Level[]`, `QuantSign[]`, or `Quant[]`,
  run `read_quant`, dequantize, transform, add residuals, reconstruct pixels, or
  change fixture output
