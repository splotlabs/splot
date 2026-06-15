# encoder-tools delta: frame-header-writer-prefix

## ADDED Requirements

### Requirement: frame-header activation-prefix writer

`splot-core` SHALL provide a writer that is the exact inverse of the § 5.18.2 frame-header
activation-prefix parser (`parse_frame_header_prefix`). For every prefix the parser can
produce, reparsing the written bits SHALL yield the original (`parse(write(x)) == x`). The
writer SHALL be additive (no parser/model edits; only a new typed writer-error variant) and
SHALL never panic: a prefix the § 5.18.2 parser could not have produced SHALL be rejected with
a typed writer error before any bit is written.

#### Scenario: the prefix round-trips across every reference form and type

- **WHEN** a parsed `FrameHeaderPrefix` is written and the bytes are reparsed
- **THEN** the reparsed prefix SHALL equal the original
- **AND** this SHALL hold for a bridge frame (inferred `cur_mfh_id == 0`), a `cur_mfh_id == 0`
  direct sequence-header reference, and a `cur_mfh_id > 0` multi-frame-header reference, across
  every frame-bearing `obu_type`.

#### Scenario: a non-canonical derived field is rejected before any bit

- **WHEN** a prefix carries an `is_*` / `startCVS` flag that disagrees with the `obu_type`
  derivation, a bridge frame with a non-zero `cur_mfh_id`, or a
  `seq_header_id_in_frame_header` / `referenced_sequence_header_id` presence that disagrees with
  the `cur_mfh_id == 0` gate
- **THEN** the writer SHALL return `WriteError::NonCanonicalFrameHeader`
- **AND** SHALL NOT write any bit (the writer buffer is left unchanged).
