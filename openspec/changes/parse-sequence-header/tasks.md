# Tasks

> Status: **proposed**. None started.

## Implementation

- [ ] Model `SequenceHeader` fields from AV2 § 5.4 (cite each).
- [ ] Parse `sequence_header_obu()` from the bounded OBU payload.
- [ ] Add directly-implied § 6.4 validator checks (if any).

## Tests and proof

- [ ] Positive / negative / EOF tests.
- [ ] Record proof in the `AV2-5.4-SEQUENCE-HEADER` row.

## Checks

- [ ] `cargo xtask ci`
