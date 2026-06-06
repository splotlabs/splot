# Change: parse-sequence-header

## Feature IDs

- `AV2-5.4-SEQUENCE-HEADER`

## Why

The sequence header carries the configuration (profile, level, color, dimensions)
needed for deeper validation and for the encoder. Today only an empty placeholder
type exists.

## Scope

- Spec sections: § 5.4 (syntax), § 6.4 (semantics).
- Crates/modules: `splot-core` (`headers`), building on `bitio`/`leb128`.

## Non-goals

- No frame header, tile group, or operating-point parsing.
- No invented fields — model only what § 5.4 defines.

## Acceptance criteria

- [ ] `SequenceHeader` models the § 5.4 fields, each spec-cited.
- [ ] Parser reads `sequence_header_obu()` from a bounded OBU payload.
- [ ] Positive, negative, and EOF tests exist.
- [ ] Validator gains any directly-implied § 6.4 checks.
- [ ] Matrix row and proof are updated.

> Status: **proposed**. Not implemented.
