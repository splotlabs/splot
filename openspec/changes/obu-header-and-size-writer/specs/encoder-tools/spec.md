# encoder-tools delta: obu-header-and-size-writer

## ADDED Requirements

### Requirement: OBU header, trailing-bits, and Annex B framing writers

`splot-core` SHALL provide writers that are the exact inverse of the § 5.2.2 OBU-header
parser and the § 5.2.3 trailing-bits parser, plus an Annex B OBU framer
(`leb128(num_bytes_in_obu)` + header + payload). For every header the parser can produce,
and every payload, reparsing the written bytes SHALL yield the original
(`read(write(x)) == x`). The writers SHALL be additive (no parser, model, or
parser-error changes) and SHALL never panic: a header or width that could not have been
produced by the parser SHALL be rejected with a typed writer error.

#### Scenario: OBU header round-trips for every type and layer

- **WHEN** a parsed OBU header is written and the bytes are reparsed
- **THEN** the reparsed header SHALL equal the original
- **AND** this SHALL hold for every `obu_type`, `obu_tlayer_id`, and (with the extension)
  every `obu_mlayer_id` / `obu_xlayer_id`.

#### Scenario: trailing_bits writes the marker bit, and rejects an empty field

- **WHEN** `write_trailing_bits(nbBits)` is called with `nbBits >= 1`
- **THEN** it SHALL write a `trailing_one_bit == 1` followed by `nbBits - 1` zero bits
- **AND** the result SHALL parse through `parse_trailing_bits` without a § 5.2.3 error
- **AND** `write_trailing_bits(0)` SHALL return a typed error rather than writing nothing.

#### Scenario: Annex B framing emits a canonical size and reparses

- **WHEN** an OBU header and payload are framed for Annex B
- **THEN** the writer SHALL emit a canonical minimal-length `leb128` size prefix
- **AND** the framed bytes SHALL parse back to the same header and payload.

#### Scenario: byte-exact holds for canonical encodings, semantic holds universally

- **WHEN** the input header is canonical and the original Annex B size prefix was minimal
- **THEN** the written bytes SHALL be byte-identical to the input
- **AND** for a non-minimal input size prefix, the re-emitted bytes MAY differ while the
  reparsed header and payload SHALL remain equal.

#### Scenario: unrepresentable headers are rejected

- **WHEN** a header's `has_header_extension` flag disagrees with `header_size_bytes`, or a
  no-extension header carries layer ids the § 5.2.2 inference could never produce
- **THEN** the writer SHALL return a typed error
- **AND** SHALL NOT panic.
