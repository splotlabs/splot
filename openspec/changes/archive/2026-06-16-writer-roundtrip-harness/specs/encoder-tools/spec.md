# encoder-tools delta: writer-roundtrip-harness

## ADDED Requirements

### Requirement: writer round-trip harness

`splot-core` SHALL provide a round-trip harness over the complete-OBU writer dispatch that, for a
parsed OBU (`ObuHeader` + `ParsedObu`) and its original payload bytes, recovers the opaque
`passthrough` bytes, writes the OBU via `write_complete_obu`, frames it with the Annex B
`leb128(num_bytes_in_obu)` size prefix, reparses, and reports whether the reparsed `ParsedObu` equals
the input. For padding the recovered passthrough SHALL be the exact `obu_padding_byte` run (whose
byte values determine the parser's split); for the length-summarized metadata blobs it MAY be any
bytes of the modeled length (the blob values are not modeled), sufficient for a semantic round-trip.
The harness SHALL never panic; for an OBU type that has no body writer yet it SHALL report an
`Unwritable` outcome rather than fail. Passthrough recovery SHALL bound its allocation by the source
payload length.

A fuzz target SHALL drive the harness over arbitrary Annex B bytes and SHALL assert that every OBU
whose payload parses to a `ParsedObu` round-trips or is `Unwritable` — never a panic and never a
round-trip failure.

#### Scenario: a parsed OBU of a written type round-trips through the harness

- **WHEN** an OBU of a written type is parsed and passed (with its original payload) to the round-trip
  harness
- **THEN** the harness SHALL recover the passthrough, write the complete OBU, reparse it, and report
  that the reparsed `ParsedObu` equals the original.

#### Scenario: an unwritten OBU type is reported as unwritable

- **WHEN** the harness is asked to round-trip a parsed OBU whose type has no body writer yet
- **THEN** it SHALL report an `Unwritable` outcome naming the missing feature, not a failure or a
  panic.

#### Scenario: arbitrary bytes never panic or fail the round-trip

- **WHEN** the fuzz target drives the harness over arbitrary Annex B input
- **THEN** every OBU whose payload parses to a `ParsedObu` SHALL round-trip or be `Unwritable`, with
  no panic.
