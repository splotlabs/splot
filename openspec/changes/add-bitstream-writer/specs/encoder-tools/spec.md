# encoder-tools delta: add-bitstream-writer

## ADDED Requirements

### Requirement: bitstream writer foundation

`splot-core` SHALL provide a `BitWriter` (and LEB128 / OBU-header writers) symmetric
with the parsers, validated by round-trip tests before any `write` stage is marked
`done`.

#### Scenario: round-trip LEB128 and OBU header

- **WHEN** the writer emits a LEB128 value or an OBU header and the result is parsed
- **THEN** the parsed value/header equals the original
