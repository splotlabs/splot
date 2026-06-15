# encoder-tools Specification

## Purpose

Encoder strategy and tooling that is not a direct AV2 syntax element: the toy intra
path, rate control, and the bitstream writer foundation. These require design before
code and produce only deliberately simple, writer-validated streams at first.

Tracked by Feature IDs: `ENC-BITSTREAM-WRITER`, `ENC-INTRA-TOY-V0`,
`ENC-RATE-CONTROL-V0`, `AV2-IVF-CONTAINER`.

**Status: in progress.** Implemented and round-trip-tested: the bit/byte **writer
primitive** layer, the **OBU header / trailing-bits / Annex B framing** writers, and the
**complete sequence header** — the general fields + decoder-model info, the
config-cascade (§ 5.4.3–5.4.8 + `seg_info`), the filter config (§ 5.4.10), the tile config
(§ 5.4.2, incl. `tile_params`), and the composing `write_sequence_header`.
`ENC-BITSTREAM-WRITER` is `partial`; `write` is `done` in the matrix for the § 4.11
descriptors (plus § 5.2.4 byte alignment), `AV2-5.2.2-OBU-HEADER` /
`AV2-5.2.3-TRAILING-BITS`, and every § 5.4 sequence-header row — the
`AV2-5.4-SEQUENCE-HEADER` umbrella `write` is now `done` — landed by the archived
`bit-writer-primitives`, `obu-header-and-size-writer`, `seq-header-writer-general`,
`seq-header-writer-configs`, and `seq-header-writer-tile` changes (see the requirements
below). Remaining: the frame/tile-group/metadata payload writers, the **Annex B**
muxer, and wiring the muxers into writer-track round-trip tests — the IVF
container write helpers already exist (`AV2-IVF-CONTAINER`, `write` = `done`); plus the
toy intra path (`ENC-INTRA-TOY-V0`) and rate control (`ENC-RATE-CONTROL-V0`). The
bootstrap `add-bitstream-writer` stub was removed, superseded by these properly-scoped
changes. The implementation matrix is the source of truth for per-row status.
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

### Requirement: sequence-header config-cascade writers

`splot-core` SHALL provide writers that are the exact inverse of the § 5.4.3–5.4.8 sequence
config parsers (partition, segment, intra, inter, scc, transform/quant/entropy) and the
§ 5.4.9 `seg_info` parser. For every model the writer accepts, reparsing the written bits with
the corresponding parser SHALL yield the original (`parse(write(x)) == x`). The writers SHALL
be additive (no parser/model/parser-error changes) and SHALL never panic: a model the parser
could not have produced SHALL be rejected with a typed writer error before any bit is written.

#### Scenario: each config round-trips across every branch

- **WHEN** a parsed config is written with the same gating inputs and the bits are reparsed
- **THEN** the reparsed config SHALL equal the original, across every conditional branch.

#### Scenario: the composite rejects a bad nested seg_info before any bit

- **WHEN** the segment config carries a `seg_info` body the parser could not have produced
- **THEN** the writer SHALL reject it before writing any bit (the leading segment flags
  included), leaving the writer buffer unchanged.

#### Scenario: a non-canonical or out-of-range field is rejected before any bit

- **WHEN** any config field exceeds its bit width, lies outside its descriptor domain, or is a
  derived/inferred value the parser would re-derive differently
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.

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

