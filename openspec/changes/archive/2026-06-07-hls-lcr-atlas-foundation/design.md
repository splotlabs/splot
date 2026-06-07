# Design: HLS layer-configuration-record and atlas foundation

## 1. Boundary

`splot-core` models the § 5.8 / § 5.9 syntax and dispatch; `splot-validate` models the
stateful § 6.4.1 / § 7.3.8 availability rules. No decoder reconstruction, writer, or
encoder work is in scope.

## 2. `leb128()` descriptor (§ 4.11.4)

`lcr_data_size[i]` is a `leb128()` element, so `BitReader` gains `read_leb128()`. It
reads up to eight 8-bit groups (via `read_bits_u8(8)`), applies the LEB128 continuation
logic, and returns `Error::InvalidLeb128` (>8 bytes / overflow) or
`Error::UnexpectedEof`. `leb128()` only appears byte-aligned in AV2; the reader does not
require alignment but is only meaningful there.

## 3. Layer configuration record (§ 5.8)

`parse_layer_config_record(reader, obu_xlayer_id)` branches on
`obu_xlayer_id == GLOBAL_XLAYER_ID` into `LcrGlobalInfo` or `LcrLocalInfo`. The full
nested syntax (`lcr_aggregate_info`, `lcr_seq_profile_tier_level_info`,
`lcr_global_payload`, `lcr_xlayer_info`, `lcr_rep_info`, `lcr_embedded_layer_info`,
`lcr_xlayer_color_info`) is parsed into strong types. Key points:

- `lcr_xlayer_map` (`f(31)`) drives the PTL and payload loops via the derived
  `LcrXLayerID[]`.
- `lcr_global_payload(n, sz)` measures parsed bits and consumes exactly `sz * 8` bits,
  including the trailing `lcr_remaining_payload_bit` bits; parsed content exceeding
  `sz * 8` is `Error::InvalidLayerConfigRecord(PayloadSizeOverflow)`.
- `byte_alignment()` uses `BitReader::byte_align_zero()`, so non-zero alignment bits
  surface as `byte-alignment/zero-bit-not-zero` like every other OBU.
- Reserved-zero fields are retained (not rejected) so the validator can warn.

## 4. Atlas segment (§ 5.9)

`parse_atlas_segment(reader)` reads `atlas_segment_id`, maps
`ats_atlas_segment_mode_idc` to `AtlasSegmentMode` (out-of-range → typed error, since
no per-mode syntax is defined), and parses the per-mode body plus
`ats_label_segment_info`. Segment and region counts are range-checked before any loop
(`< MAX_NUM_ATLAS_SEGMENTS` / `< MAX_ATLAS_COLS` / `< MAX_ATLAS_ROWS`) so a malformed
count cannot drive an unbounded parse.

## 5. Validator availability state (§ 6.4.1 / § 7.3.8)

`HlsAvailabilityStore` gains: global-LCR `id -> lcr_xlayer_map`, local-LCR
`xlayer -> {lcr_local_id}`, and local-atlas `{(xlayer, atlas_segment_id)}`. Records are
written only after a successful parse and a valid § 5.2.1 payload tail, mirroring the
sequence-header / MFH observers, and the store stays monotonic.

Diagnostics (errors gated on external HLS being disabled, matching the MFH path):

- `lcr/global-lcr-unavailable` — a local LCR's `lcr_global_id != 0` has no global LCR.
- `atlas/local-atlas-unavailable` — a local LCR's `lcr_local_atlas_id` has no local
  atlas in the same xlayer (§ 7.3.8.4 "shall be available").
- `hls/unavailable-layer-configuration-record` — a sequence header's `seq_lcr_id != 0`
  resolves to neither a local nor a global LCR.
- `lcr/global-xlayer-map-missing-xlayer` — `seq_lcr_id` resolves to a global LCR whose
  `lcr_xlayer_map` omits the sequence header's xlayer (§ 6.4.1).
- `lcr/reserved-bits-nonzero` (warning), `lcr/payload-size-overflow`,
  `atlas/segment-mode-out-of-range`, `atlas/region-dimension-out-of-range`,
  `atlas/segment-count-out-of-range` (syntax checks).

## 6. Deliberate non-checks (spec honesty)

- The global atlas (§ 7.3.8.4) is "can be available", so a missing one is not flagged.
- § 6.8 / § 6.9 define no "repeated record must be identical" requirement (unlike
  OBU_MSDO / sequence headers), so no duplicate-not-identical check is emitted.
- MFH layer-dependency-map checks are deferred: `MLayerDependencyMap` /
  `TLayerDependencyMap` are not retained by the sequence-header parser, and the task
  forbids fabricating them from max ids. A `TODO(spec: AV2-5.7-MULTI-FRAME-HEADER)`
  marks the gap.

## 7. Testing strategy

Parser unit tests for every major branch plus EOF and loop-bound rejection, a
never-panic proptest per parser, and validator tests asserting each new diagnostic
fires (and is suppressed) on hand-built annex-B streams.
