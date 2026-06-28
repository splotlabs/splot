# validator delta: streaming-validator-input

Adds a forward-only, `Read`-based streaming entry point to the validator that
bounds peak input memory to a single temporal unit while preserving the exact
diagnostics, ordering, and exit behavior of the in-memory path. Advances
`INFRA-VALIDATE-STREAMING-READER`. No new normative AV2 behavior — only how bytes
reach the existing checks.

## ADDED Requirements

### Requirement: streaming validation equivalence

`splot-validate` SHALL provide `validate_reader<R: Read>` that produces a
`ValidationReport` byte-identical to `validate_bytes` for the same bitstream — the
same set of diagnostics, in the same order, with the same offsets — and SHALL keep
`validate_bytes(&[u8])` as the stable in-memory API.

#### Scenario: reader matches in-memory on a conformant stream

- **WHEN** the same conformant stream is validated via `validate_bytes(bytes)` and
  via `validate_reader(Cursor::new(bytes))`
- **THEN** the two `ValidationReport`s are identical (diagnostics, order, offsets)

#### Scenario: reader matches in-memory on a malformed stream

- **WHEN** a truncated or malformed stream is validated via both entry points
- **THEN** both return the same typed `ValidationReport` and neither panics

### Requirement: forward-only streaming source

`validate_reader` SHALL accept any `R: Read` and SHALL NOT require `Seek`,
reassembling temporal units correctly regardless of how the reader chunks bytes.

#### Scenario: non-seekable source

- **WHEN** the input is a non-seekable reader (a pipe or `stdin`)
- **THEN** validation completes using forward reads only, with no `Seek`

#### Scenario: byte-at-a-time reads

- **WHEN** the reader returns as little as one byte per `read` call
- **THEN** temporal units are reassembled across read boundaries and the report
  matches the whole-buffer result

### Requirement: bounded input memory

The streaming validator SHALL bound peak resident input to a single temporal unit
plus a reused buffer, and SHALL NOT require the whole stream to be resident.

#### Scenario: stream much larger than one unit

- **WHEN** a stream whose total size far exceeds its largest temporal unit is
  validated via `validate_reader`
- **THEN** peak resident input stays bounded by the largest temporal unit, not the
  stream size

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
