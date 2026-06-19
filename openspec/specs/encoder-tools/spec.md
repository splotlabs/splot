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
Remaining writer surface: inter / show-existing frame-header paths, the
§ 5.18.7.11 frame-level Wiener bank (out of the current intra scope), inter
first-group tile-group composition, § 8.3 CDF selection, and the § 5.20
`decode_tile()` coded tile body. The complete-OBU dispatch now has body writers
for every parsed OBU payload variant, Annex B and IVF helpers exist, the
generic § 8.2 symbol/range encoder primitive exists, and writer-track
round-trip/fuzz/cross-tool validation coverage is tracked in the implementation
matrix. The parked
`toy-intra-encoder-v0` bootstrap change is superseded by the Baseline Encoder
Profile v1 contract; future all-intra work must be re-proposed. Rate control
(`ENC-RATE-CONTROL-V0`) remains future work. The implementation matrix is the
source of truth for per-row status.
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

### Requirement: tile-group structure writer

`splot-core` SHALL provide a writer that is the exact inverse of the § 5.19 `tile_group_obu()`
structure parser (`parse_tile_group_structure`) on the intra path: it SHALL emit
`tile_start_and_end_present_flag` (`f(1)`, only when `NumTiles > 1`), `tg_start` and `tg_end`
(`f(tileBits)`, only when `NumTiles > 1` and the flag is set, with `tileBits = TileColsLog2 +
TileRowsLog2`), and the closing `byte_alignment()` zero pad; when the tile range is inferred it
SHALL emit no range bits. For every structure the writer accepts, reparsing the written bytes SHALL
yield the original on every syntax field (`tile_start_and_end_present_flag`, `tg_start`, `tg_end`)
and a `Complete` outcome. The byte-offset parse-context fields (`header_bytes`, `payload_size`,
`outcome`) are recomputed from the surrounding OBU context and are not emitted by this writer.

The writer SHALL be additive (no model or parser-error change) and SHALL never panic: a structure the
parser could not have produced SHALL be rejected with a typed
`WriteError::NonCanonicalTileGroup` before any bit is written — including a non-`Complete`
(`Truncated`) outcome, a degenerate `NumTiles == 0` layout, a tile range that does not fit
`f(tileBits)` or violates `tg_end >= tg_start`, and a flag/range combination the parser's
inference could not have produced.

#### Scenario: the tile-group structure round-trips

- **WHEN** a parsed `TileGroupStructure` (single-tile inferred range, multi-tile with the flag clear,
  or multi-tile with an explicit `tg_start`/`tg_end`) is written and the emitted bytes are reparsed
  with the same `TileGroupLayout`
- **THEN** the reparsed structure SHALL equal the original on every syntax field with a `Complete`
  outcome, and the emitted region SHALL be byte-exact.

#### Scenario: a non-reproducible tile-group structure is rejected before any bit

- **WHEN** a structure carries a `Truncated` outcome, a degenerate layout, an out-of-range or
  inverted `tg_start`/`tg_end`, or a flag/range combination the parser's inference could not produce
- **THEN** the writer SHALL return a typed `WriteError::NonCanonicalTileGroup` and write no bit.

### Requirement: tile-group payload framing writer

`splot-core` SHALL provide a writer that is the inverse of the § 5.20.1 `tile_group_payload()`
framing parser (`parse_tile_group_framing`) on the intra (non-bridge) path: for each tile in order,
a non-last tile SHALL emit `tile_size_minus_1 = tile_size - 1` as `le(TileSizeBytes)` followed by its
coded-tile bytes, and the last tile SHALL emit its coded-tile bytes only (no size field). The
coded-tile bytes are not modeled by the parser, so the writer SHALL accept them as a per-tile
passthrough input and emit them verbatim. For every framing the writer accepts, reparsing the emitted
region with the matching `tg_start` / `tg_end` / `TileSizeBytes` SHALL yield an equal
`TileGroupFraming` (the per-tile `tile_size`s and recomputed offsets, with no defect), and the tile
bytes SHALL be byte-exact.

The writer SHALL be additive (no model or parser-error change) and SHALL never panic: a framing the
parser could not have produced — a defective framing, a bridge framing (unreconstructable
`tile_size == 0`), a tile count or passthrough length mismatch, a `TileSizeBytes` outside `1..=4`, a
zero-size tile, or a `tile_size - 1` outside `le(TileSizeBytes)` — SHALL be rejected with a typed
`WriteError` before any bit is written, and a non-byte-aligned writer SHALL be rejected with
`WriteError::WriterNotByteAligned`.

#### Scenario: the tile-group payload framing round-trips

- **WHEN** a parsed (or constructed) `TileGroupFraming` and its per-tile coded-tile bytes are written
  and the emitted region is reparsed with the same `tg_start` / `tg_end` / `TileSizeBytes`
- **THEN** the reparsed framing SHALL equal the original (sizes + recomputed offsets, no defect) and
  the coded-tile bytes SHALL be byte-exact.

#### Scenario: a non-reproducible tile-group framing is rejected before any bit

- **WHEN** a framing carries a defect, a bridge tile, a tile-data length or count mismatch, an
  out-of-range `TileSizeBytes`, a zero-size tile, or a `tile_size - 1` exceeding `le(TileSizeBytes)`
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.

### Requirement: tile-group OBU composer (first tile group)

`splot-core` SHALL provide a composing writer that emits a whole **first** intra `tile_group_obu()`
payload (`is_first_tile_group == 1`): `is_first_tile_group` `f(1) = 1` (with the inferred
`frame_header_present_flag == 1`), the embedded `frame_header()` via the existing frame-header-core
writer, the § 5.19 structure writer, and the § 5.20.1 payload framing writer, in § 5.19 read order.
The composer SHALL draft the whole payload into a scratch writer and commit it only on full success,
so any delegated sub-writer reject leaves the caller's writer untouched (reject-before-write for the
whole composition). For every model the composer accepts, reparsing the emitted payload stage by
stage (`parse_tile_group_prefix`, the frame-header core, `parse_tile_group_structure`,
`parse_tile_group_framing`) SHALL round-trip each stage's syntax fields. The composer SHALL be
additive (no model or parser-error change) and SHALL never panic.

#### Scenario: a first-tile-group OBU payload round-trips

- **WHEN** a valid first-tile-group model (frame-header core + views, § 5.19 structure, § 5.20.1
  framing + tile data) is composed and the emitted payload is reparsed stage by stage
- **THEN** each stage's syntax fields SHALL round-trip and `is_first_tile_group` SHALL reparse as `1`.

#### Scenario: an out-of-scope or non-reproducible composition is rejected before any bit

- **WHEN** the non-first (`frame_header_copy()`) continuation form is requested, or any delegated
  sub-writer rejects its model
- **THEN** the composer SHALL return a typed `WriteError` and write no bit.

### Requirement: complete-OBU writer dispatch

`splot-core` SHALL provide a writer that turns a parsed OBU (`ObuHeader` + `ParsedObu`) back into
bytes — the inverse of `dispatch_obu_payload` / `finish_obu_payload`. It SHALL write the typed payload
body via the existing per-structure writers and then the OBU tail (for a non-empty payload of an
extensible OBU type, `obu_extension_flag = 0` then `trailing_bits()`; nothing for an empty payload),
and SHALL prepend the OBU header in the complete-OBU form. For every parsed OBU of a *written* type
(temporal delimiter, sequence header, padding, metadata short, metadata group), reparsing the written
bytes SHALL yield the original `ParsedObu`. The length-summarized / opaque payloads (padding, the
metadata blobs) SHALL be supplied via a passthrough input.

For an OBU type that has no body writer yet, the dispatch SHALL return a typed
`WriteError::Unimplemented` (an honest stub) rather than panic or emit wrong bytes. The writer SHALL
be additive (no model or parser-error change beyond the new `Unimplemented` variant), SHALL be
reject-before-write (a delegated sub-writer reject or a passthrough mismatch leaves the writer
untouched), and SHALL never panic.

#### Scenario: a parsed OBU of a written type round-trips

- **WHEN** a parsed OBU of a written type (with its passthrough bytes) is written via the dispatch and
  the bytes are reparsed
- **THEN** the reparsed `ParsedObu` SHALL equal the original, and the bytes SHALL be byte-exact on the
  canonical subset.

#### Scenario: an unwritten OBU type yields a typed Unimplemented

- **WHEN** the dispatch is asked to write a `ParsedObu` variant that has no body writer yet
- **THEN** it SHALL return `WriteError::Unimplemented` and write no bit.

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

### Requirement: re-emitting a conformant stream stays validator-conformant

The `splot-core` writer SHALL re-emit an already validator-conformant bitstream (a parser-produced
stream of writable OBU types) as a stream that itself passes the `splot-validate` validator with zero
error-severity diagnostics (cross-tool agreement). This is NOT a claim that every writer output is
conformant: `write_complete_obu` faithfully serializes any encodable model, including one the
validator would reject (e.g. a parser-producible header that fails a § 6 conformance rule); the writer
reproduces its input, it does not validate it, so conformance is guaranteed only for the re-emission of
an already-conformant input. The guarantee SHALL be demonstrated by re-emitting each committed
conformant fixture that consists only of writable OBU types (temporal delimiter, padding, metadata)
through the complete-OBU writer and validating the re-emission: the re-emitted stream SHALL be
byte-exact to the canonical original and SHALL be reported as conformant
(`ValidationReport::is_conformant`).

#### Scenario: re-emitting a conformant fixture stays conformant

- **WHEN** a committed conformant fixture of writable OBU types is parsed, re-emitted through the
  writer, and validated
- **THEN** every OBU SHALL round-trip, the re-emission SHALL be byte-exact to the original, and the
  validator SHALL report zero error diagnostics.

### Requirement: buffer removal timing OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `buffer_removal_timing_obu()` (§ 5.12)
back to bytes — the inverse of `parse_buffer_removal_timing` — for both the extended-layer
(`br_ops_dependent_flag == 0`) and the operating-point-set (`br_ops_dependent_flag == 1`) forms, so
the complete-OBU dispatch round-trips this OBU type instead of returning `Unimplemented`. The writer
SHALL be reject-before-write and SHALL never panic on a constructed model: it SHALL reject an
`op_times` length that disagrees with `br_ops_cnt`, a per-operating-point `index` that disagrees with
its position, a `br_time_op` presence that disagrees with `br_decoder_model_present_op_flag`, and any
field value outside its descriptor's domain.

#### Scenario: a parsed buffer removal timing OBU round-trips

- **WHEN** a parsed `buffer_removal_timing_obu()` (either form) is written by the dispatch and the
  bytes are reparsed
- **THEN** the reparsed `BufferRemovalTiming` SHALL equal the original, byte-exact on the canonical
  subset.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given a `BufferRemovalTiming` the parser could never produce (an `op_times`
  count, `index`, or gated `br_time_op` inconsistency, or an out-of-range value)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.

### Requirement: multistream decoder operation OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `multistream_decoder_operation_obu()`
(§ 5.6) back to bytes — the inverse of `parse_msdo` — so the complete-OBU dispatch round-trips this
OBU type instead of returning `Unimplemented`. The writer SHALL be reject-before-write and SHALL
never panic on a constructed model: it SHALL reject a `multistream_large_picture_idc` presence that
disagrees with `multistream_even_allocation_flag`, a `sub_stream_count` that disagrees with
`num_streams_minus_2 + 2`, a non-zero unused sub-stream slot, and any field value outside its
descriptor's domain.

#### Scenario: a parsed MSDO OBU round-trips

- **WHEN** a parsed `multistream_decoder_operation_obu()` (even- or uneven-allocation form) is written
  by the dispatch and the bytes are reparsed
- **THEN** the reparsed `MultistreamDecoderOperation` SHALL equal the original, byte-exact on the
  canonical subset.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given a `MultistreamDecoderOperation` the parser could never produce (a gated
  `multistream_large_picture_idc`, sub-stream count, unused-slot, or out-of-range inconsistency)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.

### Requirement: operating point set OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `operating_point_set_obu()` (§ 5.10) and
its `operating_point_payload()` sub-structs (§ 5.11, § 5.11.1–5.11.5) back to bytes — the inverse of
`parse_operating_point_set` — threading the OBU header's `obu_xlayer_id` to select the global-vs-local
branch, so the complete-OBU dispatch round-trips this OBU type instead of returning `Unimplemented`.
The writer SHALL be reject-before-write and SHALL never panic on a constructed model: it SHALL reject
a `payloads` length that disagrees with `ops_cnt`, a per-element index that disagrees with its
position, any gated `Option` whose presence disagrees with its gate, and any field value outside its
descriptor's domain.

#### Scenario: a parsed operating point set OBU round-trips

- **WHEN** a parsed `operating_point_set_obu()` (reset, global, or local form) is written by the
  dispatch and the bytes are reparsed
- **THEN** the reparsed `OperatingPointSet` SHALL equal the original, byte-exact on the canonical
  subset.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given an `OperatingPointSet` the parser could never produce (a payload count,
  index, gated-`Option`, or out-of-range inconsistency)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.

### Requirement: content interpretation OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `content_interpretation_obu()` (§ 5.15)
back to bytes — the inverse of `parse_content_interpretation` (including the shared `timing_info()`)
— so the complete-OBU dispatch round-trips this OBU type instead of returning `Unimplemented`. The
writer SHALL reproduce parser-tolerated values (the `ci_reserved_2bit` and the reserved color /
aspect-ratio / scan idc values that the validator flags but the parser preserves) VERBATIM rather
than rejecting them, so a parsed model always round-trips. It SHALL be reject-before-write and SHALL
never panic on a constructed model, rejecting only the strictly decidable structural inconsistencies
(an `extended_sar` presence that disagrees with `ci_aspect_ratio_idc == 255`, a color-primaries
presence that disagrees with `ci_color_description_idc == 0`, and any field value outside its
descriptor's domain).

#### Scenario: a parsed content interpretation OBU round-trips

- **WHEN** a parsed `content_interpretation_obu()` (any combination of present sub-structs, including
  reserved idc and reserved-bit values) is written by the dispatch and the bytes are reparsed
- **THEN** the reparsed `ContentInterpretation` SHALL equal the original, byte-exact on the canonical
  subset.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given a `ContentInterpretation` the parser could never produce (an
  `extended_sar` / color-primaries gate inconsistency, or an out-of-range field)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.

### Requirement: film grain OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `film_grain_obu()` (§ 5.14, model syntax
§ 5.18.10.2) back to bytes — the inverse of `parse_film_grain` — so the complete-OBU dispatch
round-trips this OBU type instead of returning `Unimplemented`. Because the model does not store the
wire bit widths or the per-point increments, the writer SHALL canonicalize: it SHALL choose the
smallest in-range bit width that encodes every scaling-point increment, scaling value, and AR
coefficient, recompute the increments from the cumulative point values, and re-bias the AR
coefficients. Semantic round-trip (model equality) SHALL hold; byte-exactness is not guaranteed. The
writer SHALL be reject-before-write and SHALL never panic on a constructed model, rejecting the
decidable inconsistencies (non-monotonic point values, a value that fits no in-range width, count or
gated-`Option` mismatches, the forced-false flag relationships, and the derived
`sub_x`/`sub_y`/`monochrome`/`models` agreements).

#### Scenario: a parsed film grain OBU round-trips

- **WHEN** a parsed `film_grain_obu()` (any combination of slots, scaling points, and AR coefficients)
  is written by the dispatch and the bytes are reparsed
- **THEN** the reparsed `FilmGrainObu` SHALL equal the original.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given a `FilmGrainObu` the parser could never produce (a non-monotonic point,
  a count / gated-`Option` / derived-field inconsistency, or a value that fits no in-range width)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.

### Requirement: atlas segment OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `atlas_segment_info_obu()` (§ 5.9) and
its § 5.9.1–5.9.5 sub-structures back to bytes — the inverse of `parse_atlas_segment` — so the
complete-OBU dispatch round-trips this OBU type instead of returning `Unimplemented`. The writer SHALL
reproduce the § 6.9.2 descriptive segment-id assignment values verbatim (they carry no
bitstream-conformance requirement), so a parsed model always round-trips. It SHALL be
reject-before-write and SHALL never panic on a constructed model, rejecting the decidable
inconsistencies (a `mode_info` variant that disagrees with the `mode`, a `num_segments` that disagrees
with the value re-derived from the mode, gated-`Option` and count-vs-length mismatches, and
out-of-range field values).

#### Scenario: a parsed atlas segment OBU round-trips

- **WHEN** a parsed `atlas_segment_info_obu()` of any mode (single / enhanced / basic / multistream /
  multistream-with-alpha) is written by the dispatch and the bytes are reparsed
- **THEN** the reparsed `AtlasSegment` SHALL equal the original, byte-exact on the canonical subset.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given an `AtlasSegment` the parser could never produce (a mode / mode_info,
  derived-num_segments, gated-`Option`, count, or out-of-range inconsistency)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.

### Requirement: multi frame header OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `multi_frame_header_obu()` (§ 5.7) back
to bytes — the inverse of `parse_multi_frame_header`, reusing `write_seg_info` for the embedded
`seg_info()` (§ 5.4.9) — so the complete-OBU dispatch round-trips this OBU type instead of returning
`Unimplemented`. The writer SHALL reproduce the parser-tolerated `mfh_seq_header_id` / `mfh_id_minus_1`
values verbatim so a parsed model always round-trips. It SHALL be reject-before-write and SHALL never
panic on a constructed model, rejecting the decidable inconsistencies (a `mfh_apply_deblocking_filter`
array that is non-`false` when `mfh_deblocking_filter_update` is clear, the segment-info `Option`s that
disagree with `mfh_seg_info_present_flag`, an out-of-range frame-size bit width, and out-of-range field
values).

#### Scenario: a parsed multi frame header OBU round-trips

- **WHEN** a parsed `multi_frame_header_obu()` (with or without the frame-size, deblocking-update, and
  segment-info branches) is written by the dispatch and the bytes are reparsed
- **THEN** the reparsed `MultiFrameHeader` SHALL equal the original, byte-exact on the canonical
  subset.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given a `MultiFrameHeader` the parser could never produce (a forced-false
  deblocking-flag, segment-info-`Option`-vs-flag, bit-width, or out-of-range inconsistency)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.

### Requirement: layer configuration record OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `layer_config_record_obu()` (§ 5.8) and
its § 5.8.1–5.8.9 sub-structures back to bytes — the inverse of `parse_layer_config_record` — so the
complete-OBU dispatch round-trips this OBU type instead of returning `Unimplemented`. The writer SHALL
reproduce the parser-tolerated reserved-zero and descriptive values verbatim within their descriptor
domain (the `lcr_*_reserved_zero_*` fields the § 6.8 semantics say "shall be equal to 0" but the parser
retains), so an already-parsed model always round-trips. It SHALL be reject-before-write and SHALL never
panic on a constructed model, rejecting the decidable inconsistencies (a gated `Option` that disagrees
with its present flag, a `seq_ptl_infos` / `payloads` / embedded-`layers` list that disagrees with the
set-bit map it is derived from, a `lcr_global_payload` whose content plus `remaining_payload_bits` does
not equal `lcr_data_size * 8`, the embedded-layer-vs-atlas else-branch exclusivity, and out-of-range
field values).

#### Scenario: a parsed layer configuration record OBU round-trips

- **WHEN** a parsed `layer_config_record_obu()` of either scope (global or local), including the
  aggregate, PTL, payload, embedded-layer, color, and atlas sub-structures, is written by the dispatch
  and the bytes are reparsed
- **THEN** the reparsed `LayerConfigurationRecord` SHALL equal the original, byte-exact on the canonical
  subset.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given a `LayerConfigurationRecord` the parser could never produce (a
  flag-vs-`Option`, set-bit-derived list, payload-size, atlas-vs-embedded exclusivity, or out-of-range
  inconsistency)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.

### Requirement: quantizer matrix OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `quantizer_matrix_obu()` (§ 5.13 /
§ 5.4.11) back to bytes — the inverse of `parse_quantizer_matrix` — so the complete-OBU dispatch
round-trips this OBU type instead of returning `Unimplemented`, completing the writer surface for all
OBU payload types. Because the parsed model stores only the decoded coefficients (each `1..=255`), not
the wire deltas or the optional `qm_8x8_is_symmetric` / `qm_4x8_is_transpose_of_8x4` /
`qm_copy_from_previous_plane` / coefficient-repeat compressions, the writer SHALL canonicalize to the
long form — every skip flag `0`, one `svlc()` `quant_delta` per cell in 2D diagonal scan order — so the
re-emission decodes to the same coefficients (a semantic round-trip; byte-exactness is not guaranteed).
It SHALL be reject-before-write and SHALL never panic on a constructed model, rejecting the decidable
inconsistencies (a `num_planes` vs `qm_chroma_info_present_flag` disagreement, a `levels` list that
disagrees with the `qm_bit_map` set bits, an `is_default` vs `matrices` disagreement, a transform or
plane count / dimension / value-count mismatch, and a coefficient of `0`).

#### Scenario: a parsed quantizer matrix OBU round-trips

- **WHEN** a parsed `quantizer_matrix_obu()` — the reset OBU, a default level, or a user-defined level
  exercising the symmetric / transpose / copy / coefficient-repeat decode paths — is written by the
  dispatch and the bytes are reparsed
- **THEN** the reparsed `QuantizerMatrixObu` SHALL equal the original (a semantic round-trip on the
  decoded coefficients; byte-exactness is not guaranteed for the canonicalized long form).

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given a `QuantizerMatrixObu` the parser could never produce (a num-planes,
  set-bit-derived level, is-default, transform, plane-shape, value-count, or zero-coefficient
  inconsistency)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.

### Requirement: non-first tile-group continuation writer

`splot-core` SHALL provide a writer that serializes a non-first (`is_first_tile_group == 0`)
`tile_group_obu()` payload (§ 5.19 / § 5.20.1) back to bytes — the inverse of `parse_tile_group_prefix`
on the continuation path, the `frame_header_copy()` region (§ 5.18.1), `parse_tile_group_structure`,
and `parse_tile_group_framing` — so a coded frame with more than one tile group round-trips. The writer
SHALL emit `is_first_tile_group = 0`, the explicit `frame_header_present_flag`, and — when that flag is
set — the recorded first header's `NumFrameHeaderBits` `frame_header_copy()` bits verbatim, then the
shared § 5.19 structure (with no `tg_start == 0` restriction) and § 5.20.1 payload framing. It SHALL be
reject-before-write and SHALL never panic on a constructed model, rejecting a non-byte-aligned writer, a
`frame_header_present_flag` that disagrees with whether copy bits are supplied, and every reject the
delegated structure / payload sub-writers raise.

#### Scenario: a non-first tile group round-trips

- **WHEN** a non-first `tile_group_obu()` payload (with `frame_header_present_flag` set or clear, and a
  `tg_start` that may be non-zero) is written by the continuation composer and the bytes are reparsed
  into their prefix / structure / framing pieces
- **THEN** the reparsed pieces SHALL equal the originals, byte-exact on the canonical subset, and the
  `frame_header_copy()` region SHALL match the recorded first header bit-for-bit.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the continuation composer is given inputs the parser could never produce (a
  `frame_header_present_flag` vs copy-bits mismatch, or a degenerate / out-of-range structure or
  framing)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.

### Requirement: generated writer coverage document

The `xtask` automation SHALL render a writer coverage document from
`docs/IMPLEMENTATION-MATRIX.toml` for the `splot-core::write` AV2 bitstream writer surface — one row per
writable `splot-core` feature (every `splot-core` `bitstream-syntax` feature and every other
`splot-core` feature with a landed writer; writers in other crates are out of scope), with its spec
section(s), feature id, name, `write` maturity, and module — via a `cargo xtask writer-coverage`
subcommand, and `check-feature-status` SHALL regenerate and compare `docs/spec-coverage-writer.md`
(flagging a missing or out-of-date file) so it can never drift from the matrix, exactly as it already
guards the sibling `docs/FEATURE-STATUS.md` and `docs/SPEC-COVERAGE.md`.

#### Scenario: the writer coverage doc is generated and drift-guarded

- **WHEN** `cargo xtask writer-coverage --format markdown --output docs/spec-coverage-writer.md` is run
- **THEN** it SHALL write a deterministic document listing the writable features with their `write`
  status, and a subsequent `cargo xtask check-feature-status` SHALL pass; an out-of-date
  `docs/spec-coverage-writer.md` SHALL make `check-feature-status` fail with the regenerate command.

### Requirement: Writer baseline is syntax and framing, not entropy coding

The encoder tool contract SHALL distinguish the current `splot-core` writer
baseline from an encoder. The writer can emit supported parsed syntax structures
and container framing, but it SHALL NOT be treated as able to generate entropy-coded
tile payloads while § 8.3 CDF selection and the `decode_tile()` body remain unimplemented.

#### Scenario: entropy-coded tiles are not claimed

- **WHEN** encoder documentation describes current writer support
- **THEN** it states that coded tile payload generation is still a gap
- **AND** no public encoder milestone depends on fabricated coded tile bytes.

### Requirement: Closed-loop reconstruction reuse is gated

The encoder program SHALL treat `splot-recon` as available lower-level
reconstruction building blocks through a direct `splot-encode -> splot-recon`
dependency. That dependency edge SHALL NOT be treated as an integrated encoder
reconstruction loop until later input-view, closed-loop, and proof changes land.

#### Scenario: recon dependency is not public encode integration

- **WHEN** the `encoder-recon-dependency` change is reviewed
- **THEN** `splot-encode` depends on `splot-recon` only as an approved lower-level
  crate boundary
- **AND** no encoder public API reports successful encoded output because of this
  dependency alone.

#### Scenario: future closed-loop work uses the approved boundary

- **WHEN** later encoder frame-input or closed-loop reconstruction work starts
- **THEN** it may design against the approved `splot-recon` dependency
- **AND** it must still provide its own Feature IDs, tests, and matrix proof.

### Requirement: Parked toy intra change is superseded

The parked `toy-intra-encoder-v0` change SHALL NOT be resumed directly. Future
all-intra encoder work SHALL be re-proposed under the Baseline Encoder Profile v1
contract with current writer, reconstruction, validation, and conformance gates.

#### Scenario: toy encoder work restarts under a new proposal

- **WHEN** all-intra encoder implementation resumes
- **THEN** it uses a new or updated OpenSpec change tied to the Baseline Encoder
  Profile v1 contract
- **AND** the parked `toy-intra-encoder-v0` tasks remain unchecked.

### Requirement: Symbol encoder belongs to the bitstream writer foundation

`ENC-BITSTREAM-WRITER` SHALL include the generic AV2 § 8.2 symbol/range encoder
primitive as a required writer foundation before any future encoder change emits
real § 5.20 coded tile bodies. The matrix and generated writer/status docs SHALL
describe this primitive as the inverse of the existing `splot-core`
`SymbolDecoder` and SHALL keep its claim separate from § 8.3 CDF selection,
tile CDF lifecycle, syntax planning, coefficient tokenization, and coded tile
body generation.

#### Scenario: Matrix proof distinguishes primitive from tile syntax

- **WHEN** the symbol encoder primitive lands
- **THEN** `docs/IMPLEMENTATION-MATRIX.toml` and generated status docs SHALL
  record tests/fuzz evidence for the § 8.2 writer primitive under
  `ENC-BITSTREAM-WRITER`
- **AND** SHALL continue to mark coded tile body generation, coefficient syntax,
  § 8.3 CDF selection, and public encoder packet output as future or partial
  work unless those behaviors have separate runtime evidence.

#### Scenario: Public encoder behavior does not change

- **WHEN** only the symbol encoder primitive has landed
- **THEN** `splot encode` SHALL still fail honestly for lack of a coded-packet
  path
- **AND** no documentation SHALL claim Baseline Encoder Profile v1, minimal
  intra output, or broad AV2 encoder support from this primitive alone.

### Requirement: Typed runtime speed presets

The encoder runtime policy SHALL include a typed speed preset tracked by
`ENC-SPEED-PRESETS`. The preset SHALL be separate from `EncoderConfig`, SHALL have
a documented accepted numeric range, and SHALL be retained by `Context` without
creating coded packets while the encoder packet path remains unimplemented.

#### Scenario: Default runtime preset is explicit

- **WHEN** an `EncoderRuntimeConfig` is created with only a thread policy
- **THEN** it SHALL use the default speed preset
- **AND** `Context` SHALL expose that preset through runtime accessors.

#### Scenario: CLI speed is validated by the library type

- **WHEN** `splot encode --speed <n>` receives a supported preset value
- **THEN** the CLI SHALL pass the corresponding typed speed preset into
  `EncoderRuntimeConfig`
- **AND** the command SHALL continue to fail honestly because no coded packet
  path exists yet.

#### Scenario: Unsupported speed is rejected before context construction

- **WHEN** `splot encode --speed <n>` receives a value outside the accepted range
- **THEN** the value SHALL be rejected through the typed speed-preset validation
  path before encoder context construction.

#### Scenario: Speed preset is not bitstream configuration

- **WHEN** a caller chooses any accepted speed preset
- **THEN** `EncoderConfig` SHALL remain unchanged
- **AND** no documentation or API SHALL claim Baseline Encoder Profile v1 output,
  rate control, mode decision, or syntax emission from the preset framework alone.

### Requirement: Encoder residual foundation

The encoder SHALL provide a private residual-calculation stage tracked by
`ENC-RESIDUAL-FOUNDATION`. For the current 8-bit YUV420 input surface, the stage
SHALL compute signed row-major residual blocks as `source_sample -
prediction_sample` over validated borrowed input planes and caller-provided
prediction samples. The stage SHALL validate geometry and prediction shape
before returning residual data, SHALL use explicit signed arithmetic/storage,
and SHALL NOT emit syntax or create coded packets.

#### Scenario: Valid residual block computes signed differences

- **WHEN** a block rectangle inside a borrowed visible input plane and matching
  row-strided prediction samples are supplied
- **THEN** the residual stage SHALL return row-major signed samples equal to
  source minus prediction for each block sample
- **AND** the result SHALL retain the plane id and block rectangle used to
  compute it.

#### Scenario: Strided visible input and prediction rows are honored

- **WHEN** the input plane visible rectangle or prediction buffer uses stride
  padding outside the selected block
- **THEN** only samples inside the selected block SHALL contribute to the
  residual values
- **AND** padding samples SHALL NOT affect the returned row-major residuals.

#### Scenario: Invalid residual inputs are rejected

- **WHEN** the selected block is outside the visible input plane, the prediction
  stride is too small, or the prediction buffer cannot cover the selected block
- **THEN** the residual stage SHALL return a typed encoder error
- **AND** SHALL NOT return partial residual data.

#### Scenario: Residual foundation does not produce packets

- **WHEN** residual calculation is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until a later tile-body and writer integration change lands
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output from residual calculation alone.

### Requirement: Encoder forward transform foundation

The encoder SHALL provide a private forward-transform stage tracked by
`ENC-FORWARD-TRANSFORM-FOUNDATION`. For the current minimal subset, the stage
SHALL accept a 4x4 uniform signed residual block and produce a row-major 4x4
DCT_DCT coefficient block with only the DC coefficient populated. The stage SHALL
validate input shape and supported residual content before returning
coefficients, SHALL use checked arithmetic, and SHALL NOT emit syntax or create
coded packets.

#### Scenario: Uniform 4x4 residual maps to a DC-only coefficient block

- **WHEN** a 4x4 signed residual block contains the same value in every sample
- **THEN** the forward-transform stage SHALL return a 16-coefficient row-major
  block
- **AND** coefficient 0 SHALL contain the checked DC coefficient for the no-op
  quant/dequant 4x4 DCT_DCT path
- **AND** all AC coefficients SHALL be zero.

#### Scenario: No-op quant/dequant inverse reconstructs the residual block

- **WHEN** the produced coefficient block is passed unchanged through the
  `splot-recon` 4x4 DCT_DCT inverse transform path
- **THEN** the reconstructed residual block SHALL match the input uniform
  residual samples exactly
- **AND** the proof SHALL remain private test evidence rather than a public
  encoder output claim.

#### Scenario: Unsupported transform inputs are rejected

- **WHEN** the residual input is not exactly 16 samples, is non-uniform, or the
  DC coefficient calculation would overflow
- **THEN** the forward-transform stage SHALL return a typed encoder error
- **AND** SHALL NOT return partial coefficient data.

#### Scenario: Forward transform foundation does not produce packets

- **WHEN** forward transform calculation is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later quantization, tokenization, tile-body, and writer integration
  changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output from forward transform calculation alone.

### Requirement: Encoder quantization v0

The encoder SHALL provide a private fixed-quantizer stage tracked by
`ENC-QUANTIZATION-V0`. For the current minimal subset, the stage SHALL accept a
4x4 DCT_DCT DC-only transform coefficient block and a validated fixed quantizer
index, produce row-major quantized coefficients, and produce decoder-visible
dequantized coefficients through `splot-recon`. The stage SHALL validate
quantizer inputs, coefficient ranges, and arithmetic before returning data, and
SHALL NOT emit syntax or create coded packets.

#### Scenario: Fixed qindex quantizes the DC-only coefficient block

- **WHEN** a supported 4x4 DCT_DCT DC-only coefficient block and fixed qindex
  are supplied
- **THEN** the quantization stage SHALL return a 16-coefficient row-major
  quantized block
- **AND** the DC coefficient SHALL use the resolved DC quantizer
- **AND** AC coefficients SHALL use the resolved AC quantizer and remain zero
  for the current DC-only input subset.

#### Scenario: Dequant and inverse reconstruct through splot-recon

- **WHEN** the produced quantized block is dequantized by `splot-recon` and
  passed through the existing 4x4 DCT_DCT inverse transform path
- **THEN** the reconstructed residual samples SHALL match the expected current
  v0 subset evidence for fixed qindex zero
- **AND** the proof SHALL remain private test evidence rather than a public
  encoder output claim.

#### Scenario: Unsupported quantization inputs are rejected

- **WHEN** the quantizer index is outside the active bit-depth range, the
  dequant denominator is zero, a coefficient is outside the supported
  dequant-visible range, or quantization arithmetic would overflow
- **THEN** the quantization stage SHALL return a typed encoder error
- **AND** SHALL NOT return partial quantized coefficient data.

#### Scenario: Quantization v0 does not produce packets

- **WHEN** quantization calculation is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tokenization, tile-body, and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, rate control, or CLI success from quantization alone.

### Requirement: Encoder coefficient tokenization minimal

The encoder SHALL provide a private coefficient-tokenization stage tracked by
`ENC-COEFFICIENT-TOKENIZATION-MINIMAL`. For the current minimal subset, the
stage SHALL accept a top-left neutral-spatial-context 4x4 DCT_DCT DC-only
quantized block, derive coefficient scan metadata, EOB, begin position,
sign/magnitude facts, coefficient CDF q-context from qindex, and ordered
entropy token records for AV2 §5.20.7.27 and §5.20.7.28. The stage SHALL prove
those token values can be written through the in-tree AV2 §8.2 symbol encoder
with scoped CDF rows and decoded back to the same values. It SHALL NOT emit tile
payloads, coded packets, public CLI success, neighbor-derived spatial contexts,
or broad coefficient syntax beyond the declared minimal tier.

#### Scenario: All-zero block emits skip token only

- **WHEN** a supported 4x4 DCT_DCT quantized block contains only zero
  coefficients
- **THEN** the tokenization stage SHALL report EOB zero
- **AND** SHALL emit only the ordered `all_zero` entropy-token record for the
  current scoped CDF row.

#### Scenario: DC-only block emits ordered base-symbol tokens

- **WHEN** a supported 4x4 DCT_DCT quantized block contains a nonzero DC
  coefficient whose magnitude is covered by the current base-symbol tier and
  all AC coefficients are zero
- **THEN** the tokenization stage SHALL report the DC scan position, EOB, begin
  position, and sign/magnitude facts
- **AND** SHALL emit ordered entropy-token records for `all_zero`, `eob_pt_16`,
  low-frequency `coeff_base_eob`, and DC sign as required by the coefficient
  sign.

#### Scenario: Token records roundtrip through section 8.2 symbols

- **WHEN** the produced token records are written through the in-tree AV2
  section 8.2 symbol encoder using their scoped CDF rows
- **THEN** the bytes SHALL decode through the in-tree symbol decoder to the same
  ordered token values
- **AND** the proof SHALL remain private test evidence rather than packet
  output.

#### Scenario: Unsupported coefficient inputs are rejected

- **WHEN** tokenization receives an unsupported shape, transform subset,
  non-top-left spatial context, non-DC coefficient, or coefficient magnitude
  that would require syntax outside the declared minimal tier
- **THEN** the stage SHALL return a typed encoder error
- **AND** SHALL NOT return partial token data.

#### Scenario: Tokenization does not produce packets

- **WHEN** coefficient tokenization is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, rate control, or CLI success from tokenization alone.

### Requirement: Encoder closed-loop reconstruction minimal

The encoder SHALL provide a private closed-loop reconstruction stage tracked by
`ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL`. For the current minimal subset, the
stage SHALL accept a borrowed 8-bit luma 4x4 top-left source block, predict it
with AV2 §7.13.2.10 no-neighbor DC intra prediction, form and quantize a
residual through the existing private encoder residual, forward-transform, and
fixed-quantization stages, and reconstruct the decoder-visible samples through
`splot-recon` using AV2 §7.14.4/§7.14.2 dequantization, §7.15.4 inverse
transform, and §7.14.3 residual addition. The stage SHALL freeze the
reconstructed block into a `splot-recon` current-frame workspace and compute its
decoded-frame hash. Every decoder-visible step SHALL be performed by
`splot-recon`; the encoder SHALL NOT reimplement decoder-visible prediction,
dequantization, inverse transform, or residual addition. It SHALL NOT emit tile
payloads, coded packets, public CLI success, reference-frame storage, chroma or
inter reconstruction, or any reconstruction outside the declared minimal tier.

#### Scenario: Lossless qindex-zero flat block reconstructs to the source

- **WHEN** a flat 8-bit luma 4x4 top-left source block is reconstructed at
  quantizer index zero
- **THEN** the closed loop SHALL reconstruct decoder-visible samples equal to
  the source samples
- **AND** SHALL expose the reconstructed samples and a decoded-frame hash for
  the reconstructed workspace.

#### Scenario: Reconstruction and hash are deterministic

- **WHEN** the same source block and quantization parameters are reconstructed
  more than once
- **THEN** the reconstructed samples and the decoded-frame hash SHALL be
  byte-identical across runs
- **AND** the decoded-frame hash SHALL match an independently constructed
  `splot-recon` workspace filled with the reconstructed samples.

#### Scenario: Emitted coefficient decisions reconstruct identically

- **WHEN** the quantized block reconstructed by the closed loop is tokenized and
  its token records are roundtripped through the in-tree AV2 §8.2 symbol
  encoder/decoder
- **THEN** the decoded token symbols SHALL recover the exact quantized DC
  coefficient that the closed loop reconstructed from
- **AND** the reconstruction derived from that recovered coefficient SHALL equal
  the closed loop's reconstructed samples.

#### Scenario: Unsupported inputs are rejected

- **WHEN** closed-loop reconstruction receives a non-uniform source block, a
  source view whose visible size is not 4x4, an unsupported bit depth, or any
  input the underlying residual, forward-transform, or quantization stages
  reject
- **THEN** the stage SHALL return a typed encoder error
- **AND** SHALL NOT return partial reconstruction data.

#### Scenario: Closed-loop reconstruction does not produce packets

- **WHEN** closed-loop reconstruction is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, reference storage, inter support, rate control, or CLI success from
  closed-loop reconstruction alone.

### Requirement: Encoder intra-mode symbol emission minimal

The encoder SHALL provide a private intra-mode symbol-emission stage tracked by
`ENC-INTRA-MODE-SYMBOL-EMISSION`. For the current minimal subset, the stage SHALL
emit the ordered AV2 §5.20.5.5 `y_mode_set` and `y_mode_index` entropy-token
records for a DC_PRED luma block at the tile-origin neutral context, selecting the
§8.3.2 `TileYModeSetCdf` row (no context) and the `TileYModeIndexCdf` row at the
tile-origin context. The stage SHALL prove those token values can be written
through the in-tree AV2 §8.2 symbol encoder with scoped default CDF rows and
decoded back to the same values. This is the §5.20.5.5 sequence for a
non-lossless block, where `use_dpcm_y` is inferred 0 and not emitted; the stage
SHALL reject any non-tile-origin `y_mode_index` context, and SHALL emit only the
mode selector syntax and not perform the §7.13.2.10 prediction process. It SHALL
NOT emit chroma mode syntax, coefficient or all-zero symbols, lossless
`use_dpcm_y` / `dpcm_mode_y` symbols, partition syntax, tile payloads, coded
packets, public CLI success, or broad intra-mode coverage beyond the declared
minimal tier.

#### Scenario: DC_PRED block emits ordered intra-mode tokens

- **WHEN** the minimal DC_PRED luma block at the tile origin is emitted
- **THEN** the stage SHALL report exactly the ordered `y_mode_set` and
  `y_mode_index` token records
- **AND** SHALL select the `y_mode_set` CDF row with no context and the
  `y_mode_index` CDF row at the tile-origin context, both with symbol value 0.

#### Scenario: Intra-mode tokens roundtrip through section 8.2 symbols

- **WHEN** the produced intra-mode token records are written through the in-tree
  AV2 section 8.2 symbol encoder using their scoped CDF rows
- **THEN** the bytes SHALL decode through the in-tree symbol decoder to the same
  ordered token values
- **AND** the proof SHALL remain private test evidence rather than packet output.

#### Scenario: Unsupported intra-mode selectors are rejected

- **WHEN** an intra-mode CDF selector carries a `y_mode_index` context outside the
  supported range
- **THEN** the stage SHALL return a typed encoder error
- **AND** SHALL NOT return partial token data.

#### Scenario: Intra-mode emission does not produce packets

- **WHEN** intra-mode symbol emission is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, chroma mode, coefficient, or CLI success from intra-mode emission alone.

### Requirement: Encoder chroma uv_mode symbol emission

The encoder SHALL provide a private chroma `uv_mode` symbol-emission stage tracked
by `ENC-UV-MODE-SYMBOL-EMISSION`, implemented by extending the
`intra_mode_emission` module. For the current minimal subset, the stage SHALL emit
the ordered AV2 §5.20.5.6 `uv_mode` entropy-token record selecting the DC chroma
mode (`Default_Mode_List_Uv` index 0 = DC_PRED) for a non-directional DC_PRED luma
block, selecting the §8.3.2 `TileUVModeCflNotAllowedCdf` row at the
non-directional context 0. The stage SHALL prove that token value can be written
through the in-tree AV2 §8.2 symbol encoder with the scoped default CDF row and
decoded back to the same value. Per AV2 §5.20.5.3 `read_intra_uv_mode()` is called
after `read_intra_y_mode()` and before `residual()`, so `uv_mode` precedes all
coefficient symbols. This is valid only for a non-lossless block with CfL disabled
and MHCCP unavailable, where the §5.20.5.6 `use_dpcm_uv` and `is_cfl` predecessors
are not read. It SHALL NOT emit lossless `use_dpcm_uv` / `dpcm_mode_uv` or
`is_cfl` / CfL / CCTX / MHCCP syntax, coefficient or all-zero symbols, partition
syntax, tile payloads, coded packets, public CLI success, or chroma modes beyond
the declared DC minimal tier.

#### Scenario: DC chroma block emits the ordered uv_mode token

- **WHEN** the minimal DC chroma mode is emitted for a non-directional DC_PRED
  luma block
- **THEN** the stage SHALL report exactly the ordered `uv_mode` token record
- **AND** SHALL select the `TileUVModeCflNotAllowedCdf` row at the
  non-directional context 0 with symbol value 0.

#### Scenario: uv_mode token roundtrips through section 8.2 symbols

- **WHEN** the produced `uv_mode` token record is written through the in-tree AV2
  section 8.2 symbol encoder using its scoped CDF row
- **THEN** the bytes SHALL decode through the in-tree symbol decoder to the same
  token value
- **AND** the proof SHALL remain private test evidence rather than packet output.

#### Scenario: Unsupported uv_mode selectors are rejected

- **WHEN** a `uv_mode` CDF selector carries a context outside the supported
  non-directional context
- **THEN** the stage SHALL return a typed encoder error
- **AND** SHALL NOT return partial token data.

#### Scenario: uv_mode emission does not produce packets

- **WHEN** chroma `uv_mode` emission is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, coefficient, or CLI success from `uv_mode` emission alone.

### Requirement: Encoder intra-block mode trace composition

The encoder SHALL provide a private intra-block mode-trace composition stage
tracked by `ENC-INTRA-BLOCK-MODE-TRACE`, in a `block_symbol_trace` module. For
the current minimal subset, the stage SHALL compose the ordered AV2 §5.20.5.3
mode-info prefix — `y_mode_set`, `y_mode_index`, then `uv_mode` — by reusing the
merged luma and chroma mode emitters, and SHALL prove the composed sequence
writes through one in-tree AV2 §8.2 symbol encoder and decodes back through one
symbol decoder to the same ordered symbols with shared CDF state. It SHALL NOT
emit coefficient or all-zero symbols, partition syntax, tile payloads, coded
packets, public CLI success, or modes beyond the DC minimal tier.

#### Scenario: Composed trace is the ordered mode-info prefix

- **WHEN** the minimal intra DC block mode trace is composed
- **THEN** the trace SHALL be exactly the ordered luma `y_mode_set` and
  `y_mode_index` tokens followed by the chroma `uv_mode` token.

#### Scenario: Composed trace roundtrips through one section 8.2 coder

- **WHEN** the composed trace is written through one in-tree AV2 section 8.2
  symbol encoder using the scoped CDF rows
- **THEN** the bytes SHALL decode through one in-tree symbol decoder to the same
  ordered token symbols with shared CDF state
- **AND** the proof SHALL remain private test evidence rather than packet output.

#### Scenario: Mode trace does not produce packets

- **WHEN** the intra-block mode trace is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, coefficient syntax, or CLI success from the mode trace alone.

### Requirement: Encoder unified block-symbol trace with luma txb_skip

The encoder SHALL provide a private unified block-symbol trace stage tracked by
`ENC-INTRA-BLOCK-TRACE-LUMA-SKIP`, extending the `block_symbol_trace` module with
a token kind spanning the intra-mode and coefficient token kinds. For the current
minimal subset, the stage SHALL compose the ordered AV2 trace `y_mode_set`,
`y_mode_index`, `uv_mode` (§5.20.5.3 mode info), then the luma `txb_skip`
all-zero token (§5.20.7.27, the first `residual()` symbol), and SHALL prove the
combined sequence writes through one in-tree AV2 §8.2 symbol encoder and decodes
back through one symbol decoder to the same ordered symbols with shared CDF
state, routing each token to its scoped §8.3.2 CDF row from `splot-core`
defaults. It SHALL NOT emit chroma `txb_skip`, non-all-zero luma coefficients,
partition syntax, tile payloads, coded packets, public CLI success, or modes
beyond the DC minimal tier.

#### Scenario: Composed trace is the mode prefix then luma txb_skip

- **WHEN** the minimal intra DC all-zero block trace is composed
- **THEN** the trace SHALL be exactly the ordered `y_mode_set`, `y_mode_index`,
  `uv_mode` mode tokens followed by the luma `txb_skip` all-zero token.

#### Scenario: Unified trace roundtrips through one section 8.2 coder

- **WHEN** the composed trace is written through one in-tree AV2 section 8.2
  symbol encoder using the scoped mode and `txb_skip` CDF rows
- **THEN** the bytes SHALL decode through one in-tree symbol decoder to the same
  ordered token symbols with shared CDF state
- **AND** the proof SHALL remain private test evidence rather than packet output.

#### Scenario: Unsupported unified selectors are rejected

- **WHEN** the unified CDF router receives a token selector outside the supported
  minimal mode or luma `txb_skip` rows
- **THEN** the stage SHALL return a typed encoder error keyed by the token index
- **AND** SHALL NOT return partial roundtrip data.

#### Scenario: Unified trace does not produce packets

- **WHEN** the unified block-symbol trace is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, chroma coefficient syntax, or CLI success from the trace alone.

### Requirement: Encoder complete all-zero intra block trace

The encoder SHALL provide a private complete all-zero intra block trace stage
tracked by `ENC-INTRA-BLOCK-TRACE-CHROMA-SKIP`, extending the `block_symbol_trace`
module. For the current minimal subset, the stage SHALL compose the ordered AV2
trace `y_mode_set`, `y_mode_index`, `uv_mode` (§5.20.5.3), then the per-plane
`txb_skip` (`all_zero == 1`) symbols for luma, U, and V (§5.20.7.27, in
`residual()` plane order), and SHALL prove the complete six-symbol sequence writes
through one in-tree AV2 §8.2 symbol encoder and decodes back through one symbol
decoder with shared CDF state. Per §8.3.2
`TileTxbSkipCdf[is_inter || fsc_mode][txSzCtx][ctx]`, the U `txb_skip` SHALL use
the same bank as luma (the first index is `is_inter || fsc_mode` = 0 for this
intra non-FSC block, not plane type) at the §8.3.2 neutral context 6, and the V
`txb_skip` SHALL use the dedicated `TileVTxbSkipCdf` at context 0. It SHALL NOT
emit non-all-zero
coefficient symbols, CfL/CCTX, partition syntax, tile payloads, coded packets,
public CLI success, or modes beyond the DC minimal tier.

#### Scenario: Complete trace is the mode prefix then per-plane txb_skip

- **WHEN** the minimal complete all-zero intra DC block trace is composed
- **THEN** the trace SHALL be exactly the ordered `y_mode_set`, `y_mode_index`,
  `uv_mode` mode tokens followed by the luma, U, and V `txb_skip` all-zero tokens.

#### Scenario: Complete trace roundtrips through one section 8.2 coder

- **WHEN** the composed complete trace is written through one in-tree AV2 section
  8.2 symbol encoder using the scoped mode and per-plane `txb_skip` CDF rows
- **THEN** the bytes SHALL decode through one in-tree symbol decoder to the same
  ordered token symbols with shared CDF state
- **AND** the proof SHALL remain private test evidence rather than packet output.

#### Scenario: Complete trace does not produce packets

- **WHEN** the complete all-zero block trace is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, non-all-zero coefficient syntax, or CLI success from the trace alone.

