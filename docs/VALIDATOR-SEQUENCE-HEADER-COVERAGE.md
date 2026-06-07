# Sequence header coverage plan

`scope: AV2 §5.4 / §6.4`
`primary module: crates/splot-core/src/headers/sequence.rs`
`validator modules: crates/splot-validate/src/{context.rs,checks/mod.rs,validator.rs}`

## Current baseline

`AV2-5.4.1-SEQUENCE-HEADER-GENERAL` is implemented and tested. The umbrella `AV2-5.4-SEQUENCE-HEADER` is still partial because the child structures following the general section remain todo or bounded by stubs.

The next phase should avoid a single giant `SequenceHeader` patch. Implement and prove each child Feature ID separately.

## Target type shape

Recommended public/internal split:

```rust
pub struct SequenceHeader {
    pub general: SequenceHeaderGeneral,
    pub partition: Option<SequencePartitionConfig>,
    pub segment: Option<SequenceSegmentConfig>,
    pub intra: Option<SequenceIntraConfig>,
    pub inter: Option<SequenceInterConfig>,
    pub screen_content: Option<SequenceScreenContentConfig>,
    pub transform_quant_entropy: Option<SequenceTransformQuantEntropyConfig>,
    pub filter: Option<SequenceFilterConfig>,
    pub tile: Option<SequenceTileConfig>,
    pub timing: Option<TimingInfo>,
    pub decoder_model: Option<SequenceDecoderModelInfo>,
    pub film_grain_params_present: bool,
}
```

Use `Option` only when the syntax really makes the structure conditional or the implementation intentionally keeps a child as a bounded unimplemented section. Do not use `Option` to hide parse errors.

## Child section plan

| Feature ID | Spec | Initial target | Notes |
|---|---:|---|---|
| `AV2-5.4.2-SEQUENCE-TILE-CONFIG` | §5.4.2 / §6.4.2 | Partial or done, depending on `tile_params` availability | If `tile_params` is not modeled, parse `seq_tile_info_present_flag`, `allow_tile_info_change`, then stop with a feature-bound unimplemented status when tile params are actually needed. |
| `AV2-5.4.3-SEQUENCE-PARTITION-CONFIG` | §5.4.3 / §6.4.3 | Done | Straight bit-field parser plus inferred `enable_sdp`, `enable_extended_sdp`, `enable_uneven_4way_partitions`, `MaxPbAspectRatio`. |
| `AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG` | §5.4.4 / §6.4.4 | Partial first, done after `seg_info` | Parse flags and `MaxSegments`; if `seq_seg_info_present_flag == 1`, parse `seg_info(MaxSegments)` only when the segmentation tables are modeled. |
| `AV2-5.4.5-SEQUENCE-INTRA-CONFIG` | §5.4.5 / §6.4.5 | Done | Straight bit-field parser; infer `cfl_ds_filter_index = 0` for monochrome. |
| `AV2-5.4.6-SEQUENCE-INTER-CONFIG` | §5.4.6 / §6.4.6 | Partial/done by field groups | Implement still-picture branch first, then non-still feature flags. Do not implement motion/ref decoding. |
| `AV2-5.4.7-SEQUENCE-SCC-CONFIG` | §5.4.7 / §6.4.7 | Done | Straight flags and inferred values. Use AV2 names, not AV1 aliases. |
| `AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG` | §5.4.8 / §6.4.8 | Done | Syntax flags only. No transform, quantizer, or entropy coder implementation. |
| `AV2-5.4.9-SEGMENT-INFO` | §5.4.9 / §6.4.9 | Partial unless tables exist | Requires `Segmentation_Feature_Bits`, `Segmentation_Feature_Max`, and signedness. Add table/codegen plan before full done. |
| `AV2-5.4.10-SEQUENCE-FILTER-CONFIG` | §5.4.10 / §6.4.10 | Done | Syntax flags only. No filter implementation. |
| `AV2-5.4.11-USER-QM` | §5.4.11 / §6.4.11 | Partial unless table/codegen exists | Requires transform-size, scan, row/col helper tables. Do not hand-transcribe large tables without matrix proof. |
| `AV2-5.4.12-TIMING-INFO` | §5.4.12 / §6.4.12 | Done | Validate `num_units_in_display_tick > 0`, `time_scale > 0`, and bounded `num_ticks_per_picture_minus_1`. Add cross-layer consistency in validator state. |
| `AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO` | §5.4.13 / §6.4.13 | Done | Parse delays and `low_delay_mode_flag`. Keep Annex E buffering validation out of scope. |

## Parser behavior rules

1. Every field read must be tied to a Feature ID in doc comments or nearby comments.
2. EOF at every variable-width boundary returns a typed parse error, not a panic.
3. Inferred values must be stored explicitly when later validation or frame parsing needs them.
4. For fields read with `ns(n)`, reject invalid `n` through the descriptor helper.
5. For `uvlc()` fields, preserve the AV2 bound behavior from `BitReader::read_uvlc()`.
6. Child parsers return either a typed config struct or a feature-bound unimplemented status. They must not silently skip payload bits.
7. `sequence_header_obu()` must finish at a known bit position so trailing bits can be validated by the OBU dispatch layer.

## Diagnostics to wire first

| Diagnostic ID | Feature | Trigger |
|---|---|---|
| `sequence-header/timing-display-tick-zero` | `AV2-5.4.12-TIMING-INFO` | `num_units_in_display_tick == 0` |
| `sequence-header/timing-time-scale-zero` | `AV2-5.4.12-TIMING-INFO` | `time_scale == 0` |
| `sequence-header/timing-num-ticks-per-picture-out-of-range` | `AV2-5.4.12-TIMING-INFO` | `num_ticks_per_picture_minus_1 > (1 << 32) - 2` |
| `sequence-header/decoder-tick-zero` | `AV2-5.4.1-SEQUENCE-HEADER-GENERAL` | `num_units_in_decoding_tick == 0` |
| `sequence-header/user-qm-delta-out-of-range` | `AV2-5.4.11-USER-QM` | `quant_delta` outside `[-128, 127]` when implemented |
| `sequence-header/user-qm-zero-entry` | `AV2-5.4.11-USER-QM` | a written `UserQm` entry is zero when implemented |
| `sequence-header/seg-info-table-unavailable` | `AV2-5.4.9-SEGMENT-INFO` | strict validation hits table-dependent parser not yet implemented |
| `sequence-header/tile-params-unavailable` | `AV2-5.4.2-SEQUENCE-TILE-CONFIG` | strict validation hits unimplemented `tile_params` |

## Unit test matrix

For every child parser:

- positive minimal bitstream payload;
- EOF after each flag group;
- inferred-value branch test;
- inspector JSON field test if the child is exposed;
- validator diagnostic test for each local semantic rule.

Suggested test names:

```text
sequence_partition_config_reads_inferred_values
sequence_intra_config_infers_cfl_filter_for_monochrome
sequence_timing_rejects_zero_display_tick
sequence_inter_config_still_picture_branch_has_no_order_hints
sequence_filter_config_reads_all_tool_flags_without_filtering
sequence_header_child_payload_eof_never_panics
```
