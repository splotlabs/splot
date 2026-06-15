# encoder-tools delta: seq-header-writer-general

## ADDED Requirements

### Requirement: sequence-header general-fields and decoder-model writers

`splot-core` SHALL provide writers that are the exact inverse of the § 5.4.1 general
sequence-header parser (`parse_sequence_header_general`, including the dependency maps and
cropping window) and the § 5.4.13 `seq_decoder_model_info()` parser. For every model the
writer accepts, reparsing the written bytes SHALL yield the original
(`parse(write(x)) == x`). The writers SHALL be additive (no parser, model, or
parser-error changes) and SHALL never panic: a model the § 5.4 parser could not have
produced SHALL be rejected with a typed writer error before any bit is written.

#### Scenario: general fields round-trip across every branch

- **WHEN** a parsed `SequenceHeaderGeneral` is written and the bytes are reparsed with
  `parse_sequence_header_general`
- **THEN** the reparsed value SHALL equal the original
- **AND** this SHALL hold across single-picture / multi-picture, monochrome / non-mono,
  the `seq_tier` conditional, the dependency maps (multi and row-0-replicated), cropping
  present/absent, and decoder-model present/absent.

#### Scenario: a non-canonical derived value is rejected before any bit

- **WHEN** a model carries a derived or inferred value the parser would re-derive
  differently (a `seq_tier` whose gate is false, a present-flag/`Option` mismatch, a
  dependency map not reproducible from its present-flags, or a non-default cropping window
  while its flag is clear)
- **THEN** the writer SHALL return a typed `WriteError` (`NonCanonicalSequenceValue` or a
  field-domain variant)
- **AND** SHALL NOT write any bit (the writer buffer is left unchanged).

#### Scenario: dependency-map signaled bits are re-derived exactly

- **WHEN** the writer emits the `mlayer`/`tlayer` dependency maps
- **THEN** it SHALL emit the signaled bits in the parser's exact loop order (including the
  `multi` and row-0-replication rules)
- **AND** the reparsed maps SHALL equal the original derived maps.
