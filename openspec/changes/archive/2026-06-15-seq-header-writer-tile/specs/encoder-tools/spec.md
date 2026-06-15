# encoder-tools delta: seq-header-writer-tile

## ADDED Requirements

### Requirement: sequence-header filter, tile, and composing writers

`splot-core` SHALL provide writers that are the exact inverse of the § 5.4.10 sequence
filter-config parser and the § 5.4.2 sequence tile-config parser (including the shared
§ 5.18.7.3 `tile_params`), plus a composing `write_sequence_header` that emits the whole
`sequence_header_obu()` body. For every model the writer accepts, reparsing the written
bits with the corresponding parser SHALL yield the original (`parse(write(x)) == x`). The
writers SHALL be additive (no parser/model edits; only a new typed writer-error variant and
`pub(crate)` visibility on existing writer-side check helpers) and SHALL never panic: a
model the parser could not have produced SHALL be rejected with a typed writer error before
any bit is written.

#### Scenario: filter and tile configs round-trip across every branch

- **WHEN** a parsed filter or tile config is written with the same gating inputs and the
  bits are reparsed
- **THEN** the reparsed config SHALL equal the original, across every conditional branch
  (the filter `seq_sb_size`-gated unit flags and loop-restoration subfields, and the
  uniform and non-uniform tile layouts).

#### Scenario: the composing writer round-trips the whole sequence header

- **WHEN** `write_sequence_header` writes a parsed `SequenceHeader` and the bytes are
  reparsed
- **THEN** the reparsed header SHALL equal the original
- **AND** for a canonical header the written bytes SHALL be byte-identical to the input.

#### Scenario: a tile-present header at a reserved level is rejected before any bit

- **WHEN** the header signals a tile config but carries a reserved `seq_level_idx` whose
  tile layout the parser could not have produced
- **THEN** the writer SHALL return `WriteError::UnwritableSequenceHeader`
- **AND** SHALL NOT write any bit (the writer buffer is left unchanged).

#### Scenario: a gated-off non-default or out-of-range field is rejected before any bit

- **WHEN** any filter or tile field exceeds its bit width, lies outside its descriptor
  domain, or carries a non-default value while its enabling gate is clear (a value the
  parser would re-infer to a default)
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.
