# bitstream delta: streaming-validator-input

Adds a forward-only container demuxer, `TemporalUnitReader<R: Read>`, that frames
a bitstream into one temporal unit at a time over a `std::io::Read` source,
reusing the existing leb128 / Annex-B / IVF framing. Advances
`INFRA-STREAMING-TU-READER`. The reader is a new delivery mechanism for already
modeled framing; it introduces no new normative syntax.

## ADDED Requirements

### Requirement: forward-only temporal-unit reader

`splot-core` SHALL provide `TemporalUnitReader<R: Read>` that yields one temporal
unit at a time from a forward-only `Read` source, requiring no `Seek`. It SHALL
detect the container from a bounded prefix and frame units using the existing
leb128 length decoding (AV2 v1.0.0 § 4.11.6), the Annex-B OBU envelope, and the
IVF frame header. It SHALL return a typed `Error` (never a panic) on truncation or
a malformed length, reusing the existing per-unit OBU parsing without
re-implementing it.

#### Scenario: IVF stream framed per frame

- **WHEN** an IVF stream is read through `TemporalUnitReader`
- **THEN** each call yields exactly one frame's bytes (parsed from the 12-byte
  frame header's `frame_size`) until end of stream

#### Scenario: Annex-B unit spanning multiple reads

- **WHEN** the source delivers a temporal unit across several `read` calls (down to
  one byte per call)
- **THEN** the reader reassembles the full unit (using its leb128 size prefix)
  before yielding it, identical to a single-buffer read

#### Scenario: truncated unit

- **WHEN** the stream ends partway through a declared unit
- **THEN** a typed `Error` is returned and no panic occurs

### Requirement: per-unit size cap

`TemporalUnitReader` SHALL enforce a configurable per-unit byte cap and SHALL
report a typed `Error` rather than allocating a unit larger than the cap.

#### Scenario: oversized declared unit

- **WHEN** a unit's declared size exceeds the configured cap
- **THEN** a typed `Error` is returned, no oversized allocation is made, and no
  panic occurs

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
