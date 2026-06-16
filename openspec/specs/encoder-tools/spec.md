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
below). The **frame-header writer**'s intra path is now COMPLETE (sliced #4a–#4i): the § 5.18.2
activation prefix is inverted by `write_frame_header_prefix`
(`frame-header-writer-prefix`; `AV2-5.18.1-FRAME-HEADER-GENERAL` `write` = `partial`), and the
§ 5.18.4 `frame_size()` + § 5.18.3 `screen_content_params()` / `intrabc_params()` by the
`write/frame_config.rs` writers (`frame-header-writer-size-config`;
`AV2-5.18.4-FRAME-SIZE` / `AV2-5.18.3-FRAME-CONFIGURATION` `write` = `partial`), and the
§ 5.18.7.2 `tile_info()` by `write/frame_tiling.rs::write_tile_info`
(`frame-header-writer-tiling`; `AV2-5.18.7.3-TILE-PARAMS` `write` = `done`,
`AV2-5.18.7-SEGMENTATION-TILING` `write` = `partial`, reusing the shared `write_tile_params`),
and the § 5.18.6 quantization cluster by `write/frame_quant.rs`
(`frame-header-writer-quantization`; `AV2-5.18.6-QUANTIZATION` `write` = `done`), and the
§ 5.18.7.1 `segmentation_params()` by `write/frame_segmentation.rs::write_segmentation_params`
(`frame-header-writer-segmentation`; `AV2-5.18.7-SEGMENTATION-TILING` `write` stays `partial`,
reusing the shared § 5.4.9 `write_seg_info`), and the loop-filter cluster
(`deblocking_filter_params()` § 5.18.5.2, `gdf_params()` § 5.18.7.9, `cdef_params()`
§ 5.18.7.10) by `write/frame_filters.rs` (`frame-header-writer-loop-filters`;
`AV2-5.18.5-FILTERING` and `AV2-5.18.7-SEGMENTATION-TILING` `write` stay `partial`, sharing the
`gdf_per_block_is_coded` gate with the parser), and the § 5.18.7.11 `lr_params()` /
§ 5.18.7.12 `ccso_params()` cluster by `write/frame_restoration.rs`
(`frame-header-writer-restoration-ccso`; `AV2-5.18.7-SEGMENTATION-TILING` `write` stays
`partial`). The CCSO writer is byte-exact on a maintainer-approved model extension
(`ccso-offset-index-model` surfaced the `ccso_offset_idx` values) via a new `BitWriter::write_tu`
primitive; the LR writer is additive and rejects the unmodeled frame-level Wiener bank
(`frame_filters_on == true`), shipping the `frame_filters_on == false` surface only. Finally the
§ 5.18.2 intra **tail** is inverted by `write/frame_tail.rs` (`frame-header-writer-intra-tail`):
`read_tx_mode()` (§ 5.18.8.1; `AV2-5.18.8-TRANSFORM-CODING-MODES` `write` stays `partial` — the
intra surface is inverted but the inter coded arms remain), the no-bit
intra inferences, `reduced_tx_set`, the no-bit intra arm of `global_motion_params()`
(§ 5.18.9.1; `AV2-5.18.9-GLOBAL-MOTION` `write` = `partial`), and `film_grain_config()`
(§ 5.18.10.1; `AV2-5.18.10-FILM-GRAIN-STRUCTURES` `write` = `done`). The intra path is now
**composed end-to-end** by `write/frame_header_core.rs::write_frame_header_core`
(`frame-header-writer-compose`; `AV2-5.18.2-FRAME-HEADER-INFO` `write` = `partial` — the modeled
intra path is complete, the inter paths remain), the exact inverse of `parse_frame_header_core`
on the `IntraHeaderComplete` path: it writes the control-region glue (frame-type arm, long-term
ids, output flags, `order_hint`, `refresh_frame_flags`, `disable_cdf_update`) directly and
delegates every sub-structure to the #4a–#4h writers, drafting into a scratch `BitWriter` so any
reject leaves the caller's writer untouched. A full intra frame header parses → writes → reparses
byte-exactly.
The size/config and tiling slices carry maintainer-approved model/parser surfacings of
previously-discarded layout bits (`intrabc_params()` / `force_integer_mv`; the explicit-branch
`TileParams`; the CCSO `ccso_offset_idx`); the quant, segmentation, loop-filter, and intra-tail
slices are additive (their few read-but-not-stored points are redundant encodings or parser
derivations, canonicalized/re-derived like the § 5.4 leb128-minimal case).
Remaining: the inter / show-existing frame-header paths and the § 5.18.7.11 frame-level Wiener
bank (out of the current intra scope),
the tile-group/metadata payload writers, the
**Annex B** muxer, and wiring the muxers into writer-track round-trip tests — the IVF
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

### Requirement: frame-header size and configuration writers

`splot-core` SHALL provide writers that are the exact inverse of the § 5.18.4.1
`frame_size()` parser and the § 5.18.3.3 `screen_content_params()` / § 5.18.3.4
`intrabc_params()` parsers, on the intra control-region path. For every model the writer
accepts, reparsing the written bits with the corresponding parser SHALL yield the original
(`parse(write(x)) == x`), byte-exactly. The writers SHALL never panic: a model the parser
could not have produced SHALL be rejected with a typed writer error before any bit is written.

To make the `intrabc_params()` / `screen_content_params()` round-trip byte-exact rather than
merely semantic, the model and parser MAY surface the bits the modeled decode path otherwise
discards (a maintainer-approved exception to the additive / read-only-parser rule); the
surfacing SHALL NOT change the bits read (`consumed_bits` is unchanged) and SHALL preserve the
existing parser outputs.

#### Scenario: frame_size round-trips on the override and default paths

- **WHEN** a parsed `frame_size()` is written with the same gating inputs and reparsed
- **THEN** the reparsed size SHALL equal the original, for both the explicit `f(n)` override
  path and the non-override default path (which writes no bits).

#### Scenario: screen-content and intrabc params round-trip byte-exactly

- **WHEN** a parsed `screen_content_params()` / `intrabc_params()` is written with the same
  gating inputs and reparsed
- **THEN** the reparsed structure SHALL equal the original across every conditional branch
  (the `SELECT`-gated SCC/MV flags and the `frame_is_intra` / `allow_frame_max_bvp_drl_bits`
  gated intrabc fields).

#### Scenario: a non-encodable or inferred-mismatch field is rejected before any bit

- **WHEN** an overridden dimension overflows its `f(n)` field, a non-override size disagrees
  with the inferred default, an inferred SCC/MV flag disagrees with the sequence force value,
  an intrabc `Option`'s presence disagrees with its gate, or `max_bvp_drl_bits_minus_1` is
  outside the `ns(2)` domain
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.

### Requirement: frame-header tile_info writer

`splot-core` SHALL provide a writer that is the exact inverse of the § 5.18.7.2 `tile_info()`
parser, reusing the shared § 5.18.7.3 `tile_params()` writer. For every `TileInfo` the parser
can produce, reparsing the written bits SHALL yield the original (`parse(write(x)) == x`),
byte-exactly. The writer SHALL never panic: a `TileInfo` the parser could not have produced
SHALL be rejected with a typed writer error before any bit is written.

To make the explicit-branch round-trip byte-exact, the model and parser MAY surface the
derived `TileParams` the modeled path otherwise discards (a maintainer-approved exception to
the additive / read-only-parser rule); the surfacing SHALL NOT change the bits read
(`consumed_bits` is unchanged).

#### Scenario: tile_info round-trips across the reuse, explicit, and bridge paths

- **WHEN** a parsed `tile_info()` is written with the same gating inputs and reparsed
- **THEN** the reparsed `TileInfo` SHALL equal the original, across the reuse-eligible /
  inferred-reuse / explicit (uniform and non-uniform) / bridge layouts and the multi-tile
  `context_update_tile_id` / `tile_size_bytes` tail (with and without the avg-CDF gate).

#### Scenario: a non-reproducible tile_info is rejected before any bit

- **WHEN** a `TileInfo` carries a layout that does not match the `reuse_tile_params()` /
  `tile_params()` re-derivation, an inferred `reuse_tile_info` that disagrees with its gate, a
  reserved-level layout, a gated-off non-zero `context_update_tile_id`, or a
  `tile_size_bytes` whose presence / range disagrees with the syntax
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.

### Requirement: frame-header quantization writers

`splot-core` SHALL provide writers that are the exact inverse of the § 5.18.6 quantization
parsers (`read_delta_q`, `quantization_params`, `setup_qm_params`), the § 5.18.7.8
`delta_q_params`, and the § 5.18.2 lossless / QM-index tail. For every model the writer
accepts, reparsing the written bits with the corresponding parser SHALL yield the original
(`parse(write(x)) == x`). The writers SHALL be additive (no model or parser-error change; only
`pub(crate)` visibility on parser helpers) and SHALL never panic: a model the parser could not
have produced SHALL be rejected with a typed writer error before any bit is written.

Where a value has more than one parser-reachable encoding (a zero `read_delta_q`, an
all-equal QM `qm_uv_same_as_y`, the `equal_ac_dc_q` chroma-DC, or a `qm_index` selecting a
repeated level), the writer MAY emit the canonical (shortest / smallest-index) encoding; the
round-trip is then semantic universally and byte-exact on the canonical subset.

#### Scenario: each quant structure round-trips across every branch

- **WHEN** a parsed quantization / QM-setup / delta-q / lossless structure is written with the
  same gating inputs and reparsed
- **THEN** the reparsed structure SHALL equal the original, across every conditional branch
  (the QM cascade, the `diff_uv_delta` / `equal_ac_dc_q` combinations, delta-q present/absent,
  lossless coded/has-segment, and `using_qmatrix` on/off).

#### Scenario: a non-reproducible quant model is rejected before any bit

- **WHEN** a model carries a value outside its descriptor domain (`base_q_idx`, `delta_q`
  `su(7)`, a `qm_*` `f(4)`, `pic_qm_num_minus_1` `f(2)`), an inferred field that disagrees with
  its gate, a lossless array that disagrees with the `get_qindex` re-derivation, or a
  `seg_qm_level` that no QM level reproduces
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.

### Requirement: frame-header segmentation writer

`splot-core` SHALL provide a writer that is the exact inverse of the § 5.18.7.1
`segmentation_params()` parser on the intra frame-header path. For every model the writer
accepts, reparsing the written bits with the same sequence-derived (`CoreSeqSegView`) and
resolved-multi-frame-header (`MfhSegView`) inputs SHALL yield the original
(`parse(write(x)) == x`). The writer SHALL be additive (no model or parser-error change;
only a visibility-only re-export widen) and SHALL never panic: a model the parser could not
have produced SHALL be rejected with a typed writer error before any bit is written.

The writer SHALL emit fields in the parser's § 5.18.7.1 read order — `segmentation_enabled`
`f(1)` always; when enabled, `reuse_seg_info` `f(1)` only when `allowChange`; on the fresh
path the `seg_info(MaxSegments)` body via the shared § 5.4.9 segment-info writer; and no
bits on the reuse path. Every value the parser derives rather than reads —
`reuse_seg_info` when `allowChange == 0`, the reuse `features` copy, the intra-inferred
`segmentation_update_map` / `segmentation_temporal_update`, and `SegIdPreSkip` /
`LastActiveSegId` — SHALL be re-derived and validated, never coded.

#### Scenario: each segmentation branch round-trips

- **WHEN** a parsed `segmentation_params()` structure is written with the same `seg` / `mfh`
  gating inputs and reparsed
- **THEN** the reparsed structure SHALL equal the original, across every branch (disabled;
  enabled with `reuse_seg_info` inferred or coded; the fresh `seg_info()` body; and the MFH
  arm, the sequence arm, and the zero fallback for the reuse source).

#### Scenario: a non-reproducible segmentation model is rejected before any bit

- **WHEN** a model carries an inferred field that disagrees with its derivation (a
  `reuse_seg_info` not equal to `haveSegParams` when `allowChange == 0`, a reuse `features`
  table not equal to the reuse source, a `segmentation_update_map` / `segmentation_temporal_update`
  not matching the intra-path inferred constants, or a `SegIdPreSkip` / `LastActiveSegId`
  not matching the feature-table re-derivation), or a disabled model carrying any non-default
  field
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.

### Requirement: frame-header loop-filter writers

`splot-core` SHALL provide writers that are the exact inverse of the three frame loop-filter
parsers — `deblocking_filter_params()` (§ 5.18.5.2), `gdf_params()` (§ 5.18.7.9), and
`cdef_params()` (§ 5.18.7.10). For every model the writer accepts, reparsing the written bits
with the corresponding parser and the same gating inputs SHALL yield the original
(`parse(write(x)) == x`). The writers SHALL be additive (no model or parser-error change; only
`pub(crate)` visibility and a behavior-preserving gate extraction on the filtering parser) and
SHALL never panic: a model the parser could not have produced SHALL be rejected with a typed
writer error before any bit is written.

Where a value has more than one parser-reachable encoding or is derived rather than stored — a
zero `cdef_*_pri_strength` (the `cdef_*_pri_zero` form), the `cdef_*_sec_strength` `3 <-> 4`
remap, the `DfDeltaQ[i]` offset (recovered as `df_delta_q[i] - (1 << (dfParBits - 1))`), and the
`gdf_per_block` coded/inferred gate (re-derived from the `GdfGeometry`) — the writer MAY emit the
canonical encoding and re-derive the inferred value; the round-trip is then semantic universally
and byte-exact on the canonical subset.

#### Scenario: each loop-filter structure round-trips across every branch

- **WHEN** a parsed deblocking / GDF / CDEF structure is written with the same gating inputs and
  reparsed
- **THEN** the reparsed structure SHALL equal the original, across every conditional branch (the
  `CodedLossless` / enable-flag disabled returns, the deblocking MFH-update vs direct arms and
  the `DfDeltaQ` present/absent inferences, the single-picture `gdf`/`cdef` inferences, the
  `gdf_per_block` coded-vs-inferred gate, each `CdefOnSkipTxfm` arm, the `cdef` zero-flag and
  sec-strength remap, and `NumPlanes` 1 vs 3).

#### Scenario: a non-reproducible loop-filter model is rejected before any bit

- **WHEN** a model carries a value outside its descriptor domain (a `gdf` `f(2)` index, a
  `CdefDamping` / `CdefStrengths` outside its coded range, an over-wide `dfParBits`, a
  `cdef_*_sec_strength` of `3`, a `cdef_*_pri_strength` `>= 16`), an inferred field that
  disagrees with its gate (an `apply_deblocking_filter` not matching the MFH copy, a
  `gdf_per_block`/single-picture inference, an `Option` present on the wrong enabled/disabled
  branch), or a `strengths` length that disagrees with `CdefStrengths`
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.

### Requirement: frame-header loop-restoration and CCSO writers

`splot-core` SHALL provide writers that are the exact inverse of the `lr_params()` (§ 5.18.7.11)
and `ccso_params()` (§ 5.18.7.12) parsers on the intra path, plus the `tu(mx)` truncated-unary
writer primitive (§ 4.11.9) the CCSO writer needs. For every model the writer accepts, reparsing
the written bits with the corresponding parser and the same gating inputs SHALL yield the original
(`parse(write(x)) == x`). The writers SHALL be additive (no model or parser-error change; only
`pub(crate)` visibility, a behavior-preserving tool-table extraction, and the new `write_tu`
primitive) and SHALL never panic: a model the parser could not have produced SHALL be rejected with
a typed writer error before any bit is written.

Because the frame-level Wiener-bank decode (`read_wienerns_filter()`) is unmodeled — the parser
*stops* before it rather than completing — a complete `lr_params()` model can never carry
`frame_filters_on == true`. The loop-restoration writer SHALL reject any such model and SHALL write
the `frame_filters_on == false` surface (the `tool_index` reverse-lookup and the `LoopRestorationSize`
size-shift reversal). The CCSO writer SHALL reproduce the per-plane `ccso_offset_idx` loop
byte-exactly from the modeled values.

#### Scenario: each restoration/CCSO structure round-trips across every branch

- **WHEN** a parsed `lr_params()` (a `Parsed` outcome) or `ccso_params()` structure is written with
  the same gating inputs and reparsed
- **THEN** the reparsed structure SHALL equal the original, across every conditional branch (the
  disabled returns, the per-plane tool selection and `LoopRestorationSize` size signaling for each
  `SbSize`, the CCSO single-picture / frame-flag / `ccso_bo_only` / quant-step inferences, the
  `ccso_offset_idx` loop, and `NumPlanes` 1 vs 3).

#### Scenario: an unwritable or non-reproducible model is rejected before any bit

- **WHEN** an `lr_params()` model carries a plane with `frame_filters_on == true` (the unmodeled
  Wiener bank), a `LoopRestorationSize` shift unreachable for the frame `SbSize`, or a disabled
  restoration tool; or a `ccso_params()` model carries an `Option` present on the wrong branch, an
  out-of-domain index, or a `ccso_offset_idx` length that disagrees with `maxEdgeInterval² * maxBand`
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.

### Requirement: frame-header intra-tail writers

`splot-core` SHALL provide writers that are the exact inverse of the § 5.18.2 intra-tail
parsers — `read_tx_mode()` (§ 5.18.8.1), `film_grain_config()` (§ 5.18.10.1), and the composed
intra tail (`read_tx_mode()`, the no-bit intra inferences, `reduced_tx_set`, the no-bit intra arm
of `global_motion_params()`, and `film_grain_config()`). For every model the writer accepts,
reparsing the written bits with the corresponding parser and the same gating inputs SHALL yield
the original (`parse(write(x)) == x`). The writers SHALL be additive (no model or parser-error
change) and SHALL never panic: a model the parser could not have produced SHALL be rejected with a
typed writer error before any bit is written.

The composed intra-tail writer SHALL validate the whole tail — including the `tx_mode` lossless
consistency and the `film_grain_config()` model — before writing the first bit, so a reject can
never leave a partial buffer.

#### Scenario: each intra-tail structure round-trips across every branch

- **WHEN** a parsed `read_tx_mode()` / `film_grain_config()` / intra tail is written with the same
  gating inputs and reparsed
- **THEN** the reparsed structure SHALL equal the original, across every branch (the lossless
  `ONLY_4X4` inference vs `tx_mode_select`; the three-way `apply_grain` gate and the `fgm_id` /
  `grain_seed` presence; the five no-bit intra inferences and `reduced_tx_set`).

#### Scenario: a non-reproducible intra-tail model is rejected before any bit

- **WHEN** a model carries an `ONLY_4X4` `tx_mode` on a non-lossless frame (or a non-`ONLY_4X4`
  on a lossless one), a `true` for any no-bit intra inference, an `apply_grain` disagreeing with
  its inferred value, a wrong `fgm_id` / `grain_seed` presence, or an `fgm_id` / `reduced_tx_set`
  outside its field
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.

### Requirement: composing intra frame-header writer

`splot-core` SHALL provide a composing writer `write_frame_header_core` that is the exact
inverse of `parse_frame_header_core` on the path that reaches
`FrameHeaderParseStatus::IntraHeaderComplete`. It SHALL emit the whole intra frame header in
§ 5.18.2 order — the activation prefix, the control-region glue bits (the frame-type arm, the
long-term-id reads, the output-control flags, `frame_size_override_flag`, `order_hint`,
`refresh_frame_flags`, and `disable_cdf_update`), and every sub-structure (frame size,
screen-content, intrabc, tile, quantization, segmentation, QM setup, delta-Q, lossless,
deblocking, GDF, CDEF, loop-restoration, CCSO, and the tail) — by delegating each sub-structure
to its existing writer. For every model the writer accepts, reparsing the written bits with
`parse_frame_header_core` and the same gating inputs SHALL yield the original on every structural
field (`parse(write(x)) == x`). Because the delegated sub-writers canonicalize redundant
descriptor encodings (e.g. `write_read_delta_q` emits the shorter `delta_coded == 0` form for a
`delta_q == 0` model the parser also accepts as the coded-zero form, like the § 5.18.6 quant /
§ 5.4 leb128-minimal cases), this round-trip is semantic universally and byte-exact only on the
canonical subset; the informational derived `consumed_bits` is excluded from the equality.

The writer SHALL accept ONLY a model whose `status == IntraHeaderComplete` (with
`frame_is_intra` set, the required fields present, and no partial loop-restoration parse); any
other model — an inter / switch / TIP / bridge / show-existing-frame header, a non-complete
status, or a model with a missing required field — SHALL be rejected with a typed writer error
before any bit is written. The composition SHALL never leave a partial buffer: a reject at any
step SHALL leave `bit_len() == 0`.

#### Scenario: a complete intra frame header round-trips

- **WHEN** an intra frame header that the parser turned into an `IntraHeaderComplete`
  `FrameHeaderCore` is written with `write_frame_header_core` and the same sequence / MFH inputs
- **THEN** the written bytes SHALL reparse to a `FrameHeaderCore` equal to the original on every
  structural field (and byte-exact when the original used the canonical sub-encodings), across the
  intra frame types (single-picture Key, closed-/open-loop key, intra-only), lossless and
  non-lossless, grain present/absent, single- and multi-tile, and `cur_mfh_id` 0 and > 0.

#### Scenario: a non-intra-complete model is rejected before any bit

- **WHEN** a model carries a non-`IntraHeaderComplete` status, a missing required field, a
  partial loop-restoration parse, or a show-existing-frame / inter header
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.

### Requirement: metadata OBU writers

`splot-core` SHALL provide writers that are the exact inverse of the § 5.17
`metadata_short_obu()` and `metadata_group_obu()` parsers, including `metadata_unit()` and the 11
typed `metadata_*` payloads. For every model the writer accepts, reparsing the written bytes with
the corresponding parser SHALL yield the original on every structural field
(`parse(write(x)) == x`). The writers SHALL be additive (no model or parser-error change) and
SHALL never panic: a model the parser could not have produced SHALL be rejected with a typed
writer error before any bit is written.

The fully-modeled payloads SHALL be byte-exact. For the length-summarized payloads (ITU-T T.35,
ICC profile, user-data-unregistered, and the reserved/unknown raw payload), which the model
carries by length only, the writer SHALL accept the opaque payload bytes as a separate input and
emit them verbatim — byte-exact without a model change. The short-OBU `metadata_type` leb128
SHALL be reproduced byte-exactly from its modeled `metadata_type_leb128_bytes`, and the group-unit
`muh_payload_size` leb128 length SHALL be derived from the modeled `muh_header_size` so it too is
byte-exact. The group OBU's `metadata_type` and `metadata_unit_cnt_minus_1` leb128 byte counts are
not modeled and the byte-granular unit padding / discarded header-extension bytes are not carried,
so for those the round-trip SHALL be semantic universally and byte-exact on the canonical
(minimal-`leb128`, zero-padded) subset. A model the parser could not have produced SHALL be
rejected with a typed writer error before any bit (a single additive, writer-only
`NonCanonicalMetadata` reject variant; the parser/decoder error model is untouched).

#### Scenario: each metadata OBU round-trips

- **WHEN** a parsed `metadata_short_obu()` or `metadata_group_obu()` is written with the same
  passthrough payload bytes and reparsed
- **THEN** the reparsed structure SHALL equal the original, across every payload type, both OBU
  forms, and the `muh_cancel_flag` arms; and the bytes SHALL be byte-exact on the canonical subset.

#### Scenario: a non-reproducible metadata model is rejected before any bit

- **WHEN** a model carries a field outside its descriptor domain, a passthrough length that
  disagrees with the modeled `payload_len`, or a `muh_*` gated field that disagrees with its
  cancel-flag / header-size derivation
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.

