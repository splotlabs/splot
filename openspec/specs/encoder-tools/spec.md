# encoder-tools Specification

## Purpose

Encoder strategy and tooling that is not a direct AV2 syntax element: the toy intra
path, rate control, and the bitstream writer foundation. These require design before
code and produce only deliberately simple, writer-validated streams at first.

Tracked by Feature IDs: `ENC-BITSTREAM-WRITER`, `ENC-INTRA-TOY-V0`,
`ENC-RATE-CONTROL-V0`, `AV2-IVF-CONTAINER`.

**Status: in progress.** The bit/byte **writer primitive** layer is implemented and
round-trip-tested: `ENC-BITSTREAM-WRITER` is `partial` and the § 4.11 descriptor
`write` stages (plus § 5.2.4 byte alignment) are `done` in the matrix, landed by the
archived `bit-writer-primitives` change (see the "bit-writer primitive inverses"
requirement below). The remaining writer surface (OBU header, payload writers, the
Annex B / IVF muxers), the toy intra path (`ENC-INTRA-TOY-V0`), and rate control
(`ENC-RATE-CONTROL-V0`) remain planned; the parked `openspec/changes/add-bitstream-writer`
stub is superseded by `bit-writer-primitives` for the primitive layer. The
implementation matrix is the source of truth for per-row status.

## Requirements
### Requirement: writer symmetry

When implemented, the bitstream writer SHALL be symmetric with the parsers: anything
the writer emits SHALL parse back to an equal structure (round-trip tests) before
the writer stage is marked `done` in the matrix.

#### Scenario: round-trip

- **WHEN** the writer emits a syntax element
- **THEN** parsing the bytes back yields an equal structure

### Requirement: legal-stream-only encoding

When implemented, the toy encoder SHALL emit only syntax that is implemented in the
writer and accepted by `splot validate`. It SHALL NOT fabricate AV2 syntax.

#### Scenario: toy intra output

- **WHEN** a single intra frame is encoded by the toy path
- **THEN** the output validates clean under `splot validate`

### Requirement: Encoder and decoder container choices

Future encoder and decoder APIs SHALL treat raw Annex B and IVF as explicit,
supported bitstream container choices. IVF output SHALL use the shared
`splot-core` writer helpers rather than CLI-only code.

#### Scenario: Future IVF encoder output

- **WHEN** a future encoder writes IVF output
- **THEN** it SHALL use the shared IVF writer
- **AND** each frame payload SHALL remain parseable by the shared Annex B parser.

#### Scenario: Future decoder IVF input

- **WHEN** a future decoder receives IVF input
- **THEN** it SHALL use the shared input-format parser
- **AND** SHALL receive the same OBU stream ordering as the validator.

### Requirement: bit-writer primitive inverses

`splot-core` SHALL provide a `BitWriter` that is the exact inverse of every
`BitReader` descriptor primitive — `f(n)`, `su(n)`, `uvlc()`, `svlc()`, `le(n)`,
`leb128()`, `ns(n)`, `rg(n)`, and zero-pad byte alignment — packing bits
most-significant-bit first. For every value the writer accepts, parsing the written
bits back with the corresponding reader primitive SHALL yield the original value
(`read(write(x)) == x`).

The writer SHALL be additive: it depends on the reader and model read-only and SHALL
NOT modify parser, model, or parser-error code. It SHALL never panic on any value or
width; values or widths the corresponding reader could never produce SHALL be
rejected with a typed writer error.

#### Scenario: every primitive round-trips

- **WHEN** `BitWriter` writes a value with a primitive (`f(n)`, `su(n)`, `uvlc()`,
  `svlc()`, `le(n)`, `leb128()`, `ns(n)`, or `rg(n)`) and the bytes are parsed back
  with the matching `BitReader` primitive
- **THEN** the parsed value SHALL equal the original value
- **AND** the property SHALL hold across the full valid value space of each primitive
  (verified by property tests).

#### Scenario: unencodable values are rejected, not panicked

- **WHEN** a caller asks the writer to encode a value outside a descriptor's domain
  (a value too wide for a fixed field, a signed value outside `su(n)` range, a width
  outside a descriptor's allowed range, or an `rg(n)` quotient that would not
  terminate within 32 bits)
- **THEN** the writer SHALL return a typed writer error
- **AND** SHALL NOT panic.

#### Scenario: byte alignment pads with zero bits

- **WHEN** the writer aligns a partial byte to the next byte boundary
- **THEN** it SHALL pad with zero bits
- **AND** the result SHALL parse back through `byte_align_zero()` without an
  alignment error.

