# OPS/BRT diagnostics registry

Stable diagnostic IDs for the `ops-brt-hls-foundation` phase
(`OBU_OPERATING_POINT_SET` § 5.10-§ 5.11, `OBU_BUFFER_REMOVAL_TIMING` § 5.12). Every
diagnostic carries a stable `rule_id`, a `severity`, a `spec_section`, the OBU byte
offset, and a concise message with the offending field values.

## Implemented OPS diagnostics

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `ops/local-reserved-bits-nonzero` | error | 6.10.2 | A local OPS has `ops_reserved_2bits != 0`. |
| `ops/mlayer-info-idc-reserved` | error | 6.10.2 | A global OPS has `ops_mlayer_info_idc == 3` (reserved). |
| `ops/ptl-reserved-bits-nonzero` | error | 6.10.4 | An `ops_seq_profile_tier_level_info()` has `ops_ptl_reserved_2bits != 0`. |
| `ops/payload-size-mismatch` | error | 6.10.2 | A payload's computed `opsBytes` differs from its declared `ops_data_size`. |
| `ops/inherited-op-index-out-of-range` | error | 6.10.2 | `ops_embedded_op_index >= ops_cnt[obu_xlayer_id][refID]`, or, for a same-OPS reference, `>= j` (the included extended layer). |

## Implemented BRT diagnostics

| Rule ID | Severity | Section | Condition |
|---|---|---|---|
| `brt/unavailable-operating-point-set` | error | 7.3.8.5 | An OPS-dependent BRT's `(obu_xlayer_id, br_ops_id)` resolves to no active in-band OPS, and external HLS is disabled. |
| `brt/ops-count-mismatch` | error | 6.11 | An OPS-dependent BRT's `br_ops_cnt` differs from the referenced active OPS `ops_cnt`. |

## Deferred diagnostics (tracked, not emitted this phase)

| Rule ID | Section | Why deferred |
|---|---|---|
| `ops/inherited-ops-unavailable` | 6.10.2 | A cross-OPS inheritance reference to an unavailable OPS is not flagged, to avoid false positives under external HLS. |
| `ops/mlayer-dependency-missing` | 6.10.7 | Needs the activated sequence header's `MLayerDependencyMap`, which the sequence-header model does not expose. |
| `ops/tlayer-dependency-missing` | 6.10.7 | Needs the activated `TLayerDependencyMap` (as above). |
| `brt/global-ordering-position` | 7.3.7 | § 7.3.7 does not list BRT among the global temporal-unit prefix OBUs; a hard ordering error needs decoder-model / random-access state. |

## Parser errors vs validator diagnostics

The parser returns a typed `Error` only when input ends before a required field, a
variable-length code is malformed, or a closing `byte_alignment()` pad bit is non-zero.
Every conformance violation that leaves the bitstream parseable (reserved bits,
reserved `ops_mlayer_info_idc`, `opsBytes` mismatch, inheritance bounds, BRT
references) is a validator diagnostic with an OBU byte offset, so the parser keeps
going and the finding carries field values.
