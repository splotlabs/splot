# Validator diagnostics registry

`status: enforced`
`owner: validator`
`purpose: the canonical, CI-enforced list of every diagnostic rule id the validator emits`

> **Canonical diagnostic registry (CI-enforced).** The tables in the marker-delimited
> region below (between the `diagnostics-registry:begin` and `:end` HTML comments) are the
> single source of truth for validator diagnostic rule IDs.
> `cargo xtask check-diagnostic-registry` (run inside `cargo xtask ci`, tracked as
> `XTASK-DIAGNOSTIC-REGISTRY`) fails if any rule-id literal in `crates/splot-validate/src`
> is missing from these tables, or if a table lists an ID that is not present in the source.
> The gate enforces the rule-ID *set*; the `Severity` and `Section` columns are maintained by
> hand. The planned-diagnostics backlog in
> [`VALIDATOR-ROADMAP.md`](./VALIDATOR-ROADMAP.md) feeds into this registry as its
> diagnostics land. The extractor lives in `xtask/src/diagnostic_registry.rs`.

Diagnostics are the validator product. Every finding carries:

- a stable `rule_id`;
- a `severity` (`error`, `warning`, `info`);
- an optional `spec_section`;
- optional byte offset and bit offset;
- a human-readable message;
- test coverage when the owning feature is marked proven in `docs/IMPLEMENTATION-MATRIX.toml`.

<!-- diagnostics-registry:begin -->

## Emitted diagnostics

Every rule ID below is emitted by `crates/splot-validate/src`, grouped by namespace. The
`Section` column cites the AV2 v1.0.0 conformance section the check derives from.

### `atlas/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `atlas/duplicate-input-stream-id` | error | § 6.9.6 | ats_input_stream_id / ats_msi_input_stream_id values are not unique |
| `atlas/local-atlas-unavailable` | error | § 7.3.8.4 | local LCR references lcr_local_atlas_id with no available local atlas (external disabled) |
| `atlas/multistream-requires-global-xlayer` | error | § 6.9 | multistream atlas mode does not use GLOBAL_XLAYER_ID |
| `atlas/region-dimension-out-of-range` | error | § 6.9.3.1 | an atlas region dimension is out of range |
| `atlas/segment-count-out-of-range` | error | § 6.9.6 | atlas segment count is out of range |
| `atlas/segment-mode-out-of-range` | error | § 6.9 | ats_atlas_segment_mode_idc is out of range |

### `bitstream/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `bitstream/parse-error` | error | varies | a payload parse / EOF / malformed-descriptor error (spec section set per call site) |

### `brt/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `brt/ops-count-mismatch` | error | § 6.11 | BRT br_ops_cnt differs from the active OPS ops_cnt |
| `brt/unavailable-operating-point-set` | error | § 7.3.8.5 | BRT `(obu_xlayer_id, br_ops_id)` has no active in-band OPS and no matching external-HLS OPS declaration (per-object, unlike the coarse external-disabled gate on the other availability checks) |

### `byte-alignment/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `byte-alignment/zero-bit-not-zero` | error | § 6.2.4 | a byte_alignment() padding zero bit is non-zero |

### `content-interpretation/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `content-interpretation/aspect-ratio-idc-out-of-range` | error | § 6.14 | ci_aspect_ratio_idc exceeds 16 when not equal to 255 |
| `content-interpretation/chroma-sample-position-out-of-range` | error | § 6.14 | ci_chroma_sample_position top or bottom exceeds 5 |
| `content-interpretation/repeated-ci-not-identical` | error | § 6.14 | repeated CI OBU for same xlayer/mlayer in CVS carries different information |
| `content-interpretation/reserved-bits-nonzero` | warning | § 6.14 | ci_reserved_2bit is non-zero (decoder-ignored producer anomaly) |

### `decoder-model/`

No AVM differential oracle exists for the `decoder-model/` rules: AVM parses both
decoder-model syntax sites but never enforces or consumes the signaled buffer-delay
values (its only consumer hardcodes 70000/20000), so proof for these rules is
hand-crafted unit vectors only — `avm_diff` is never claimed for them.

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `decoder-model/buffer-delay-sum-changed` | error | § 6.10.5 | the same (obu_xlayer_id, ops_id, operating-point index) is redefined within one coded video sequence with no intervening OPS reset, both signalings explicitly carry decoder-model info, and the ops_decoder_buffer_delay + ops_encoder_buffer_delay sum differs (non-conforming under § 6.4.13 / § 6.10.5 sum-constancy on every candidate "video sequence" reading) |
| `decoder-model/buffer-delay-sum-changed-across-cvs` | warning | § 6.4.13 / § 6.10.5 | an explicitly signaled buffer-delay sum changes across a coded-video-sequence or OPS-reset boundary — the activated sequence header's seq_decoder_model_info() sum across a CLK boundary (frame-confirmed activations only; emitted with spec_section § 6.4.13) or an OPS sum across a CVS/reset boundary for the same triple (emitted with spec_section § 6.10.5); advisory because the § 6.4.13 / § 6.10.5 "video sequence" scope is unspecified |

### `film-grain/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `film-grain/chroma-idc-out-of-range` | error | § 6.13 | fgm_chroma_idc exceeds 3 |
| `film-grain/chroma-points-not-paired` | error | § 6.17.10.2 | in 4:2:0, num_cb_points and num_cr_points are not both zero or both non-zero |
| `film-grain/duplicate-slot-in-coded-frame-unit` | error | § 6.13 | a film grain slot is updated more than once in the same coded frame unit |
| `film-grain/scaling-point-not-increasing` | error | § 6.17.10.2 | a scaling point value is not strictly increasing or not less than 256 |
| `film-grain/scaling-points-out-of-range` | error | § 6.17.10.2 | num_y/cb/cr_points exceeds 14 |
| `film-grain/update-flags-zero` | error | § 6.13 | fgm_update_flags is 0 |

### `frame-header/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `frame-header/bridge-ref-index-out-of-range` | error | § 6.17.2 | bridge_frame_ref_idx is not less than NumRefFrames |
| `frame-header/context-update-tile-id-out-of-range` | error | § 6.17.7.2 | context_update_tile_id is not less than TileCols * TileRows |
| `frame-header/cur-mfh-id-out-of-range` | error | § 6.17 | cur_mfh_id is not less than MAX_MFH_NUM |
| `frame-header/frame-size-exceeds-sequence-max` | error | § 6.17.4.1 | derived FrameWidth/FrameHeight exceeds active sequence maximum |
| `frame-header/frame-to-refresh-out-of-range` | error | § 6.17.2 | refresh_frame_flags sets a reference slot at or beyond NumRefFrames |
| `frame-header/intra-only-refresh-all-slots` | error | § 6.17.2 | INTRA_ONLY_FRAME with NumRefFrames>1 refreshes every slot |
| `frame-header/mfh-mlayer-dependency-missing` | error | § 7.3.8.7 | frame header references an MFH whose recorded MfhMLayerId the frame's obu_mlayer_id does not depend on (§ 6.17.2: MLayerDependencyMap[obu_mlayer_id][MfhMLayerId] != 1) |
| `frame-header/mfh-tlayer-dependency-missing` | error | § 7.3.8.7 | frame header references an MFH whose recorded MfhTLayerId the frame's layer does not depend on (§ 6.17.2: TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][MfhTLayerId] != 1) |
| `frame-header/qm-plane-count-mismatch` | error | § 6.17.6.2 | a qm_y/qm_u/qm_v custom-QM reference whose recorded QmNumPlanes differs from the sequence NumPlanes |
| `frame-header/ras-requires-long-term-frame-id-bits` | error | § 6.4.6 | OBU_RAS_FRAME present but active sequence long_term_frame_id_bits == 0 |
| `frame-header/ref-long-term-id-reserved` | error | § 6.17.2 | a ref_long_term_id[i] equals the reserved (1<<long_term_frame_id_bits)-1 |
| `frame-header/refresh-frame-flags-zero-on-deferred-output` | error | § 6.17.2 | immediate_output_frame==0 with refresh_frame_flags==0 |
| `frame-header/seq-header-id-out-of-range` | error | § 6.17 | seq_header_id_in_frame_header is not less than MAX_SEQ_NUM |
| `frame-header/still-picture-requires-key-frame` | error | § 6.17.2 | still_picture sequence without KEY_FRAME and immediate_output_frame==1 |
| `frame-header/switch-or-ras-mlayer-dependency-not-self-contained` | error | § 6.4.1 | OBU_SWITCH / OBU_RAS_FRAME has MLayerDependencyMap[obu_mlayer_id][m] != 0 for some embedded layer m != obu_mlayer_id |
| `frame-header/tile-cols-out-of-range` | error | § 6.17.7.2 | frame tile_info() derives TileCols greater than MAX_TILE_COLS |
| `frame-header/tile-rows-out-of-range` | error | § 6.17.7.2 | frame tile_info() derives TileRows greater than MAX_TILE_ROWS |

### `hls/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `hls/external-hls-disabled` | warning | § 7.3.8.1 | a referenced sequence header is unavailable in-band and external HLS is disabled (advisory) |
| `hls/multiple-active-sequence-headers` | error | § 7.3.6 | a frame-confirmed activation of a different seq_header_id follows an earlier frame-confirmed activation within the same coded video sequence (no intervening CLK) |
| `hls/repeated-sequence-header-not-identical` | error | § 7.3.6 | activated sequence header is repeated within CVS with different payload bytes |
| `hls/unavailable-layer-configuration-record` | error | § 7.3.8.3 | seq_lcr_id resolves to no available local or global LCR (external disabled) |
| `hls/unavailable-multi-frame-header` | error | § 7.3.8.7 | frame header references a cur_mfh_id with no available multi-frame header (external HLS disabled) |
| `hls/unavailable-sequence-header` | error | § 7.3.8.6 | frame header references a sequence header id that is unavailable |

### `ivf/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `ivf/invalid-header-length` | error | IVF | IVF header length is smaller than the 32-byte baseline header |
| `ivf/invalid-signature` | error | IVF | container signature is not `DKIF` when parsing as IVF |
| `ivf/truncated-frame-header` | error | IVF | input ends before a complete 12-byte IVF frame header |
| `ivf/truncated-frame-payload` | error | IVF | input ends before the declared IVF frame payload is complete |
| `ivf/truncated-header` | error | IVF | input ends before the declared IVF header is complete |

### `lcr/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `lcr/dependent-xlayers-flag-nonzero` | warning | § 6.8.2 | lcr_dependent_xlayers_flag is set (decoder-ignored) |
| `lcr/global-id-out-of-range` | error | § 6.8.2 | lcr_global_config_record_id is 0 (must be 1..7) |
| `lcr/global-lcr-unavailable` | error | § 7.3.8.3 | local LCR references an unavailable global LCR (external HLS disabled) |
| `lcr/global-xlayer-map-missing-xlayer` | error | § 6.4.1 | sequence header xlayer is not set in the referenced global LCR lcr_xlayer_map |
| `lcr/local-id-zero` | error | § 6.8.3 | lcr_local_id equals 0 |
| `lcr/mlayer-dependency-missing` | error | § 6.8.9 | activated LCR lcr_mlayer_map includes an embedded layer without a layer the activated sequence header's MLayerDependencyMap requires |
| `lcr/payload-size-overflow` | error | § 6.8.6 | layer config record declared payload size overflows |
| `lcr/reserved-bits-nonzero` | warning | § 6.8 | a layer config record reserved-zero field is non-zero (decoder-ignored) |
| `lcr/tlayer-dependency-missing` | error | § 6.8.9 | activated LCR lcr_tlayer_map includes a temporal layer without a layer the activated sequence header's TLayerDependencyMap requires |
| `lcr/xlayer-map-empty` | error | § 6.8.2 | lcr_xlayer_map is 0 (must be 1..(1<<31)-1) |

### `metadata/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `metadata/group-header-underflow` | error | § 6.16.3 | metadata group header underflows the payload |
| `metadata/group-layer-idc-reserved` | warning | § 6.16.3 | group-unit muh_layer_idc is 4..7 (reserved for AOMedia use) |
| `metadata/group-mlayer-map-below-obu-mlayer` | error | § 6.16.3 | muh_mlayer_map sets a bit below obu_mlayer_id |
| `metadata/group-reserved-bits-nonzero` | warning | § 6.16.3 | muh_reserved_zero_2bits is non-zero (decoder-ignored) |
| `metadata/group-unit-count-too-large` | error | § 6.16.3 | metadata group unit count is too large |
| `metadata/group-xlayer-map-global-bit-set` | error | § 6.16.3 | bit 31 of muh_xlayer_map is set |
| `metadata/hdr-cll-repeat-content-differs` | error | § 6.16.5 | HDR CLL metadata units in a CVS associated with a common embedded layer (per § 6.16.3 layer targeting) have different content |
| `metadata/hdr-mdcv-repeat-content-differs` | error | § 6.16.6 | HDR MDCV metadata units in a CVS associated with a common embedded layer (per § 6.16.3 layer targeting) have different content |
| `metadata/persistence-idc-reserved` | warning | § 6.16.3 | muh_persistence_idc is 4..7 (reserved for AOMedia use) |
| `metadata/scan-type-ci-scan-type-mismatch` | error | § 6.16.10 | mps_pic_struct_type requires a ci_scan_type_idc that differs from a non-zero CI value established in the CVS scope at or after the layer's most recent random access point (§ 7.3.8.11) |
| `metadata/scan-type-ci-scan-type-unestablished` | warning | § 6.16.10 | scan-type metadata present but no CI established a non-zero ci_scan_type_idc in the CVS scope (default is 0, § 7.3.8.11) |
| `metadata/scan-type-equal-picture-interval-required` | error | § 6.16.10 | mps_pic_struct_type 7/8 while CI timing_info established in the current § 7.3.8.11 epoch signals equal_picture_interval 0 |
| `metadata/scan-type-pic-struct-group-inconsistent` | error | § 6.16.10 | mps_pic_struct_type values in the same CVS fall into more than one Table 6.18 group |
| `metadata/scan-type-pic-struct-reserved` | error | § 6.16.10 | mps_pic_struct_type exceeds 12 (reserved) |
| `metadata/short-layer-idc-out-of-range` | error | § 6.16.2 | muh_layer_idc >= 3 for OBU_METADATA_SHORT |
| `metadata/temporal-point-info-not-short` | error | § 6.16.11 | METADATA_TYPE_TEMPORAL_POINT_INFO appears outside OBU_METADATA_SHORT |
| `metadata/timecode-hours-out-of-range` | error | § 6.16.7 | timecode hours_value exceeds 23 |
| `metadata/timecode-minutes-out-of-range` | error | § 6.16.7 | timecode minutes_value exceeds 59 |
| `metadata/timecode-seconds-out-of-range` | error | § 6.16.7 | timecode seconds_value exceeds 59 |
| `metadata/unit-payload-underflow` | error | § 6.16.1 | metadata unit payload underflows declared size |

### `mfh/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `mfh/id-out-of-range` | error | § 5.7 | mfhId is not less than MAX_MFH_NUM (16) |
| `mfh/seq-header-id-out-of-range` | error | § 6.4.1 | mfh_seq_header_id is not less than MAX_SEQ_NUM (16) |
| `mfh/sequence-header-unavailable` | error | § 7.3.8.6 | multi-frame header references an unavailable mfh_seq_header_id |

### `msdo/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `msdo/non-global-layer-id` | error | § 6.6 | OBU_MSDO does not use tlayer==0, mlayer==0, xlayer==GLOBAL_XLAYER_ID |
| `msdo/too-many-streams` | error | § 6.6 | num_streams_minus_2 exceeds 2 |

### `obu-header/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `obu-header/base-layer-only-types` | error | § 6.2.2 | a base-layer-only OBU type has non-zero obu_tlayer_id or obu_mlayer_id |
| `obu-header/extension-flag-not-zero` | error | § 6.2.1 | obu_extension_flag is not 0 in this spec version |
| `obu-header/global-xlayer-allowed-types` | error | § 6.2.2 | GLOBAL_XLAYER_ID used by an OBU type that does not permit it |
| `obu-header/global-xlayer-required` | error | § 6.2.2 | OBU type requiring GLOBAL_XLAYER_ID uses a non-global obu_xlayer_id |
| `obu-header/global-xlayer-requires-base-layers` | error | § 6.2.2 | GLOBAL_XLAYER_ID used with non-zero obu_mlayer_id or obu_tlayer_id |
| `obu-header/reserved-obu-type` | info | § 6.2.2 | a reserved obu_type is present (ignored by conformant decoders) |
| `obu-header/temporal-layer-zero-only-types` | error | § 6.2.2 | key/switch/RAS frame type has non-zero obu_tlayer_id |

### `obu-order/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `obu-order/duplicate-temporal-delimiter` | error | § 7.3.7 | a second global temporal delimiter with no intervening OBU |
| `obu-order/global-hls-after-coded-layer` | error | § 7.3.7 | a global HLS prefix OBU appears after a coded extended layer unit |
| `obu-order/padding-non-global-outside-coded-layer` | error | § 7.3.7 | OBU_PADDING outside a coded extended layer unit is not GLOBAL_XLAYER_ID |
| `obu-order/temporal-unit-missing-delimiter` | error | § 7.3.7 | an OBU appears before a global temporal delimiter starts the temporal unit |
| `obu-order/xlayer-order-not-ascending` | error | § 7.3.7 | coded extended layer units are not in ascending obu_xlayer_id order |

### `obu-reserved/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `obu-reserved/all-zero-payload` | error | § 5.3 | reserved OBU has non-empty payload that is entirely zero |

### `ops/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `ops/inherited-op-index-out-of-range` | error | § 6.10.2 | inherited ops_embedded_op_index out of range for referenced OPS |
| `ops/local-reserved-bits-nonzero` | error | § 6.10.2 | local OPS ops_reserved_2bits is non-zero |
| `ops/mlayer-dependency-missing` | error | § 6.10.7 | explicit ops_mlayer_map includes an embedded layer without a layer the activated sequence header's MLayerDependencyMap requires |
| `ops/mlayer-info-idc-reserved` | error | § 6.10.2 | global OPS ops_mlayer_info_idc == 3 (reserved) |
| `ops/payload-size-mismatch` | error | § 6.10.2 | ops_data_size differs from the parsed payload byte count |
| `ops/ptl-reserved-bits-nonzero` | error | § 6.10.4 | ops_ptl_reserved_2bits is non-zero |
| `ops/tlayer-dependency-missing` | error | § 6.10.7 | explicit ops_tlayer_map includes a temporal layer without a layer the activated sequence header's TLayerDependencyMap requires |

### `padding/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `padding/all-zero-payload` | error | § 5.16 | OBU_PADDING payload is entirely zero (no non-zero byte) |
| `padding/invalid-trailing-bits` | error | § 5.16 | padding OBU trailing_bits are invalid |

### `qm/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `qm/duplicate-level-between-frames` | error | § 6.12 | same quantizer matrix level specified twice between coded frames |
| `qm/duplicate-reset-between-frames` | error | § 6.12 | QM OBU with qm_bit_map==0 is not the first QM OBU between coded frames |
| `qm/quant-delta-out-of-range` | error | § 6.4.11 | a quantizer-matrix quant delta value is out of range |

### `sequence-header/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `sequence-header/bit-depth-out-of-range` | error | § 6.4.1 | coded bit depth is out of range |
| `sequence-header/chroma-format-out-of-range` | error | § 6.4.1 | chroma format value out of range |
| `sequence-header/crop-bottom-out-of-range` | error | § 6.4.1 | crop_bottom is out of range |
| `sequence-header/crop-left-out-of-range` | error | § 6.4.1 | crop_left is out of range |
| `sequence-header/crop-right-out-of-range` | error | § 6.4.1 | crop_right is out of range |
| `sequence-header/crop-top-out-of-range` | error | § 6.4.1 | crop_top is out of range |
| `sequence-header/seq-header-id-out-of-range` | error | § 6.4.1 | seq_header_id is out of its valid range |
| `sequence-header/seq-max-mlayer-count-out-of-range` | error | § 6.4.1 | seq_max_mlayer_count is out of range |
| `sequence-header/timing-display-tick-mismatch` | error | § 6.4.12 | num_units_in_display_tick differs across embedded layers in same CVS |
| `sequence-header/timing-display-tick-zero` | error | § 6.4.12 | num_units_in_display_tick is zero |
| `sequence-header/timing-equal-picture-interval-mismatch` | error | § 6.4.12 | equal_picture_interval differs across embedded layers in same CVS |
| `sequence-header/timing-num-ticks-mismatch` | error | § 6.4.12 | num_ticks_per_picture_minus_1 differs across embedded layers in same CVS |
| `sequence-header/timing-num-ticks-per-picture-out-of-range` | error | § 6.4.12 | num_ticks_per_picture_minus_1 is out of range |
| `sequence-header/timing-num-units-zero` | error | § 6.4.1 | timing num_units value is zero |
| `sequence-header/timing-time-scale-mismatch` | error | § 6.4.12 | time_scale differs across embedded layers in same CVS |
| `sequence-header/timing-time-scale-zero` | error | § 6.4.12 | time_scale is zero |

### `sequence-state/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `sequence-state/distinct-mlayer-count-exceeds-seq-max` | error | § 6.4.1 | the distinct obu_mlayer_id count in an extended layer's coded video sequence exceeds the active sequence header's SeqMaxMlayerCnt |
| `sequence-state/mlayer-exceeds-max` | error | § 6.2.2 | obu_mlayer_id exceeds active sequence max_mlayer_id |
| `sequence-state/monotonic-output-order-mismatch` | error | § 6.4.1 | extended layers inside a coded multistream video sequence disagree on monotonic_output_order_flag |
| `sequence-state/no-active-sequence-header` | error | § 7.3.8 | OBU uses an xlayer before an active sequence header is available |
| `sequence-state/tlayer-exceeds-max` | error | § 6.2.2 | obu_tlayer_id exceeds active sequence max_tlayer_id |
| `sequence-state/unknown-sequence-header-id` | error | § 7.3.8 | the active seq_header_id for an xlayer is unavailable |

### `tile-params/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `tile-params/nonuniform-cols-do-not-cover-frame` | error | § 6.17.7.3 | non-uniform tile column widths do not sum to sbCols |
| `tile-params/nonuniform-rows-do-not-cover-frame` | error | § 6.17.7.3 | non-uniform tile row heights do not sum to sbRows |
| `tile-params/tile-cols-out-of-range` | error | § 6.17.7.2 | TileCols exceeds MAX_TILE_COLS |
| `tile-params/tile-rows-out-of-range` | error | § 6.17.7.2 | TileRows exceeds MAX_TILE_ROWS |

### `trailing-bits/`

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `trailing-bits/empty` | error | § 6.2.3 | trailing_bits() found empty payload where a trailing one-bit was required |
| `trailing-bits/missing-one-bit` | error | § 6.2.3 | trailing_bits() is missing the required leading 1 bit |
| `trailing-bits/zero-bit-not-zero` | error | § 6.2.3 | a trailing_zero_bit after the one-bit is non-zero |

## Check registry identifiers

These are `Check::id()` registry identifiers, **not** diagnostics emitted verbatim: a failed
parse of the corresponding OBU surfaces a specific `bitstream/parse-error`,
`trailing-bits/*`, or `byte-alignment/*` diagnostic via `syntax_error_diagnostic()` instead.
They are listed here so the registry's documented set equals the rule-id literals present in
the source (the `Parse §` column is the section the OBU's syntax is parsed from).

| Registry ID | Parse § | Routed through |
|---|---|---|
| `atlas/syntax` | § 5.9 | `syntax_error_diagnostic()` |
| `brt/syntax` | § 5.12 | `syntax_error_diagnostic()` |
| `content-interpretation/syntax` | § 5.15 | `syntax_error_diagnostic()` |
| `film-grain/syntax` | § 5.14 | `syntax_error_diagnostic()` |
| `lcr/syntax` | § 5.8 | `syntax_error_diagnostic()` |
| `metadata/syntax` | § 5.17 | `syntax_error_diagnostic()` |
| `mfh/syntax` | § 5.7 | `syntax_error_diagnostic()` |
| `msdo/syntax` | § 6.6 | `syntax_error_diagnostic()` |
| `ops/syntax` | § 5.10 | `syntax_error_diagnostic()` |
| `padding/syntax` | § 5.16 | `syntax_error_diagnostic()` |
| `qm/syntax` | § 5.13 | `syntax_error_diagnostic()` |
| `sequence-header/syntax` | § 5.4 | `syntax_error_diagnostic()` |
| `trailing-bits/empty-syntax-obu-payload` | § 5.2.3 | `syntax_error_diagnostic()` |

<!-- diagnostics-registry:end -->

## Severity guidance

- `error` — a conformance violation that leaves the bitstream parseable (reserved bits, an
  out-of-range field, an unavailable referenced HLS object, an ordering violation).
- `warning` — a decoder-ignored reserved field or a capability-gated condition that is not a
  hard violation (the `*/reserved-bits-nonzero` checks, `hls/external-hls-disabled`).
- `info` — informative only (e.g. a reserved `obu_type` a conformant decoder ignores).

A parse failure — input ending before a required field, a malformed variable-length code, or
a non-zero closing `byte_alignment()` pad bit — is converted into a `bitstream/parse-error`
(or a specific `trailing-bits/*` / `byte-alignment/*`) diagnostic rather than a panic. IVF
container failures use `ivf/*` diagnostics. Malformed payloads and containers are reported
with byte offsets instead of silently accepted.

## Planned / not yet emitted

The following namespaces are reserved for future validator work and are intentionally **absent
from the enforced registry above** because nothing emits them yet:

- `tile-group/` — frame-data / tile payload boundary checks (needs full frame/tile parsing).
- `hls-availability/` — a dedicated high-level-syntax availability namespace; today the landed
  availability checks live under `hls/` (see the registry above).
- `obu-payload/`, `annex-a/` — strict-mode payload and Annex A profile/level constraints.
  (The `decoder-model/` namespace has landed — see the registry tables above — but is limited to
  signaled buffer-delay sum-constancy; Annex E decoder-schedule simulation remains future.)

Design sketches and phase plans for these live in the planned-diagnostics backlog of
[`VALIDATOR-ROADMAP.md`](./VALIDATOR-ROADMAP.md).
When a planned diagnostic lands, add its rule ID to the enforced tables above (the CI gate
will require it) and update `DIAGNOSTIC_PREFIXES` in `xtask/src/feature_status.rs` if it
introduces a new namespace.

## Intentional non-checks (spec honesty)

Conformance points deliberately not flagged, in two groups.

**Structurally unobservable or not a spec requirement** — these stay non-checks:

- The global atlas (§ 7.3.8.4) is "can be available", so a missing global atlas is not an error.
- § 6.8 / § 6.9 define no "repeated record must be identical" rule, so no LCR/atlas
  duplicate-not-identical diagnostic is emitted (unlike `OBU_MSDO` / sequence headers).
- The § 6.4.11 requirement that no value written into `UserQm` equals 0
  (`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md`, "User defined QM semantics") is not
  a diagnostic: the § 5.4.11 parse makes a zero entry unrepresentable — the running quant
  starts at 32, a computed `quant2 == 0` selects the coefficient-repeat path (writing the
  prior non-zero value), and mirror/copy paths replicate already non-zero values — so the
  validator cannot observe a violation.

**Deferred pending infrastructure** — planned in the
[`VALIDATOR-ROADMAP.md`](./VALIDATOR-ROADMAP.md) backlog, not fabricated today:

- The § 6.10.7 / § 6.8.9 / § 7.3.8.7 dependency-map agreement checks (landed as
  `ops/*-dependency-missing`, `lcr/*-dependency-missing`,
  `frame-header/mfh-*-dependency-missing`) run only against a **decidable activated
  in-band** sequence header — one confirmed by a parsed frame-header reference, or
  the OBU-order fallback while it is the sole in-band header — and the maps are
  never fabricated from defaults, max layer IDs, or an ambiguous multi-header
  fallback guess. Each group's no-false-positive gate matches what external HLS
  could shadow: the OPS checks are suppressed when external HLS declares any
  sequence header, the LCR checks whenever external HLS is enabled at all (an
  unmodeled external *local* LCR would win the § 6.4.1 resolution), and the MFH
  checks are skipped when the referenced sequence header does not resolve in-band.
  The § 6.8.9 pairing binds the header's § 6.4.1 *association*, snapshotted at each
  observation of that header (an LCR "present prior to this sequence header"): a
  later-arriving LCR is not retroactively paired, and a record redefined after the
  header's latest observation is not the associated one. An OPS/LCR entry whose
  extended layer never activates a decidable in-band header is not checked.
- An unresolved cross-OPS inheritance reference is not flagged (`ops/inherited-ops-unavailable`
  is reserved) because the reference may be supplied through external HLS.

## Testing expectations per diagnostic

Every new diagnostic requires:

1. one positive case that does **not** emit it;
2. one negative case that emits it;
3. a byte offset when available;
4. a spec section in the diagnostic;
5. a proof entry in `docs/IMPLEMENTATION-MATRIX.toml` when the owning feature stage is `done`;
6. a CLI JSON test for at least one diagnostic per new namespace.

## Diagnostic JSON compatibility

Diagnostic JSON is part of the product. Do not rename existing fields without a compatibility
plan; adding fields is acceptable when the CLI tests are updated. `splot validate --json`
prints a report object whose `diagnostics` array holds one object per finding, serialized
from `Diagnostic` in `crates/splot-validate/src/diagnostic.rs`: `severity` is the
capitalized variant name (`"Error"`, `"Warning"`, `"Info"`), and an unset `spec_section`,
`byte_offset`, or `bit_offset` serializes as `null`. One finding looks like:

```json
{
  "rule_id": "sequence-header/chroma-format-out-of-range",
  "spec_section": "6.4.1",
  "severity": "Error",
  "byte_offset": 42,
  "bit_offset": 3,
  "message": "chroma_format_idc must be <= 3, found 4"
}
```

No `feature_id` field is emitted: feature IDs live in code comments, tests, and
`docs/IMPLEMENTATION-MATRIX.toml`, not in the diagnostic payload.
