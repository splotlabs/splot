# Validator diagnostic registry additions for next phase

`scope: next validator coverage phase`
`rule: every diagnostic has a stable id, severity, section, offset, and human-readable message`

## Existing families to preserve

Do not rename existing diagnostic prefixes without a migration note:

- `bitstream/`
- `obu-header/`
- `obu-reserved/`
- `trailing-bits/`
- `byte-alignment/`
- `sequence-header/`
- `sequence-state/`
- `obu-order/`

## New sequence-header diagnostics

| ID | Severity | Section | Feature | Trigger |
|---|---|---|---|---|
| `sequence-header/decoder-tick-zero` | error | §6.4.1 | `AV2-5.4.1-SEQUENCE-HEADER-GENERAL` | `num_units_in_decoding_tick == 0` when decoder model info is present. |
| `sequence-header/timing-display-tick-zero` | error | §6.4.12 | `AV2-5.4.12-TIMING-INFO` | `num_units_in_display_tick == 0`. |
| `sequence-header/timing-time-scale-zero` | error | §6.4.12 | `AV2-5.4.12-TIMING-INFO` | `time_scale == 0`. |
| `sequence-header/timing-num-ticks-per-picture-out-of-range` | error | §6.4.12 | `AV2-5.4.12-TIMING-INFO` | `num_ticks_per_picture_minus_1 > (1 << 32) - 2`. |
| `sequence-header/timing-display-tick-mismatch` | error | §6.4.12 | `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` | Present timing values differ across embedded layers in the same coded video sequence. |
| `sequence-header/timing-time-scale-mismatch` | error | §6.4.12 | `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` | `time_scale` differs across embedded layers in the same coded video sequence. |
| `sequence-header/timing-equal-picture-interval-mismatch` | error | §6.4.12 | `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` | `equal_picture_interval` differs across embedded layers in the same coded video sequence. |
| `sequence-header/timing-num-ticks-mismatch` | error | §6.4.12 | `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` | `num_ticks_per_picture_minus_1` differs across embedded layers in the same coded video sequence. |
| `sequence-header/user-qm-delta-out-of-range` | error | §6.4.11 | `AV2-5.4.11-USER-QM` | `quant_delta` is outside `[-128, 127]`. |
| `sequence-header/user-qm-zero-entry` | error | §6.4.11 | `AV2-5.4.11-USER-QM` | Computed `UserQm` entry is zero. |
| `sequence-header/seg-info-table-unavailable` | warning or error in strict mode | §5.4.9 | `AV2-5.4.9-SEGMENT-INFO` | Parser reaches `seg_info()` but required tables are intentionally not implemented. |
| `sequence-header/tile-params-unavailable` | warning or error in strict mode | §5.4.2 | `AV2-5.4.2-SEQUENCE-TILE-CONFIG` | Parser reaches `tile_params()` but shared tile parser is intentionally not implemented. |

## New HLS/state diagnostics

| ID | Severity | Section | Feature | Trigger |
|---|---|---|---|---|
| `hls/unavailable-sequence-header` | error | §7.3.8 | `AV2-7.3.8-HLS-AVAILABILITY` | Referenced sequence header is not available in-band or externally. |
| `hls/external-hls-disabled` | error | §7.3.8 | `AV2-7.3.8-HLS-AVAILABILITY` | A validation path requires external HLS but the option is disabled. |
| `hls/repeated-sequence-header-not-identical` | error | §7.3.8 | `AV2-7.3.8-HLS-AVAILABILITY` | Activated sequence header repeats with different payload bytes. |
| `hls/multiple-active-sequence-headers` | error or warning until CLK parsing exists | §7.3.8 | `AV2-7.3.8-HLS-AVAILABILITY` | More than one active sequence header is observed for an extended layer without a modeled reset. |
| `sequence-state/monotonic-output-order-mismatch` | error | §6.4.1 | `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` | Extended layers in a coded multistream video sequence disagree on `monotonic_output_order_flag`. |
| `sequence-state/distinct-mlayer-count-exceeds-seq-max` | error | §6.4.1 | `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` | Count of distinct `obu_mlayer_id` values exceeds `SeqMaxMlayerCnt`. |

## New OBU ordering diagnostics

| ID | Severity | Section | Feature | Trigger |
|---|---|---|---|---|
| `obu-order/duplicate-temporal-delimiter` | error | §7.3.7 | `AV2-7.3.7-TEMPORAL-UNIT-ORDER` | A temporal unit has more than one global temporal delimiter. |
| `obu-order/missing-temporal-delimiter` | error | §7.3.7 | `AV2-7.3.7-TEMPORAL-UNIT-ORDER` | Existing `temporal-unit-missing-delimiter` may be kept instead; do not duplicate names. |
| `obu-order/global-hls-after-metadata-suffix` | error | §7.3.7 | `AV2-7.3.7-TEMPORAL-UNIT-ORDER` | Global HLS appears after suffix metadata once metadata suffix parsing exists. |
| `obu-order/non-global-hls-before-coded-layer` | error | §7.3.7 | `AV2-7.3.7-TEMPORAL-UNIT-ORDER` | Non-global HLS appears in an invalid temporal-unit region. |

## MSDO/MFH diagnostics

| ID | Severity | Section | Feature | Trigger |
|---|---|---|---|---|
| `msdo/non-global-layer-id` | error | §6.6 | `AV2-5.6-MSDO` | MSDO is not global/base-layer/base-temporal. |
| `msdo/too-many-streams` | error | §6.6 | `AV2-5.6-MSDO` | `num_streams_minus_2 > 2`. |
| `msdo/sub-xlayer-duplicate` | error | §6.6 | `AV2-5.6-MSDO` | Duplicate `sub_xlayer_id` where uniqueness is required by the implemented semantic rule. Add only after confirming exact spec wording. |
| `mfh/seq-header-id-out-of-range` | error | §6.7 | `AV2-5.7-MULTI-FRAME-HEADER` | `mfh_seq_header_id >= MAX_SEQ_NUM`. |
| `mfh/id-out-of-range` | error | §6.7 | `AV2-5.7-MULTI-FRAME-HEADER` | `mfh_id_minus_1 + 1 >= MAX_MFH_NUM`. |
| `mfh/sequence-header-unavailable` | error | §7.3.8 | `AV2-7.3.8-HLS-AVAILABILITY` | MFH references a sequence header that is not available. |

## Naming rules

- Use lowercase kebab-case after the slash.
- Prefix by the conceptual owner, not the Rust module name.
- Do not encode numeric values into the ID.
- Do not rename an existing ID to make a table prettier.
- When a check is intentionally partial, the message should say what dependency is missing.
