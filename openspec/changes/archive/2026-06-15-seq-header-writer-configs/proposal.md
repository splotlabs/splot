# Change: seq-header-writer-configs

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.4.3-SEQUENCE-PARTITION-CONFIG`, `AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG`,
  `AV2-5.4.5-SEQUENCE-INTRA-CONFIG`, `AV2-5.4.6-SEQUENCE-INTER-CONFIG`,
  `AV2-5.4.7-SEQUENCE-SCC-CONFIG`, `AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG`,
  `AV2-5.4.9-SEGMENT-INFO` (each advances its `write` stage `todo -> done`)

## Why

The sequence-header config cascade (§ 5.4.3–5.4.8) is the bulk of the sequence header.
This change lands the inverse of the six config parsers plus `seg_info` (§ 5.4.9), on
top of the merged general-fields writer. The remaining slice (`seq-header-writer-tile`)
adds the filter and tile configs and the composing `write_sequence_header`.

## What changes

- Add `crates/splot-core/src/write/seq_config.rs`: public `write_sequence_partition_config`,
  `write_sequence_segment_config`, `write_sequence_intra_config`,
  `write_sequence_inter_config`, `write_sequence_scc_config`,
  `write_sequence_transform_quant_entropy_config` — each the inverse of its public parser,
  threading the same gating inputs (`monochrome`, `single_picture`, …) and validating fully
  up front.
- Add `crates/splot-core/src/write/segment.rs`: `write_seg_info` + `check_seg_info_encodable`.
- The segment-config writer pre-validates the nested `seg_info` body before any flag is
  written, so the composite rejects before any bit. The module stays **additive** — no
  parser/model/error edits, and no new `WriteError` variant (the existing variants suffice).

## Validator impact

None. No new diagnostics; the validator is unchanged.

## Non-goals

- No filter config (§ 5.4.10), tile config / `tile_params` (§ 5.4.2), or the top-level
  `write_sequence_header` — the final `seq-header-writer-tile` sub-change.
- No encoder rate decisions; no public `encode` CLI.

## Impact

- Crate: `crates/splot-core` (additive `write` module only).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (+ regenerated `docs/FEATURE-STATUS.md`).
