# Change: operating-point-set-writer

## Feature IDs

- `AV2-5.10-OPERATING-POINT-SET` (write: `todo` → `done`)
- `AV2-5.11-OPERATING-POINT-PAYLOAD`, `AV2-5.11.1-OPS-AGGREGATE-INFO`, `AV2-5.11.2-OPS-SEQ-PTL-INFO`,
  `AV2-5.11.3-OPS-DECODER-MODEL-INFO`, `AV2-5.11.4-OPS-COLOR-INFO`, `AV2-5.11.5-OPS-MLAYER-INFO`
  (the payload + aggregate / PTL / decoder-model / color / mlayer sub-structs; all `write` → `done`)
- `ENC-BITSTREAM-WRITER` (the third of the unwritten OBU-type body writers)

## Why

Continue moving the parser-modeled OBU types from `Unimplemented` to round-trippable in the
complete-OBU dispatch. `operating_point_set_obu()` (§ 5.10) is the next target: flat fixed-width and
`uvlc` fields (no delta coding), but a deeply nested, heavily-gated structure (the global-vs-local
`obu_xlayer_id` branch and the per-operating-point `operating_point_payload()` with its gated
aggregate / PTL / color / mlayer sub-structs). It is the largest single OBU writer so far.

## What changes

- **Writer** (`crates/splot-core/src/write/operating_point_set.rs`, new; additive, no model change):
  `write_operating_point_set(writer, ops, obu_xlayer_id)` — the inverse of
  `parse_operating_point_set` + `parse_operating_point_payload` and the § 5.11.1–5.11.5 sub-struct
  parsers, field order preserved. It threads `obu_xlayer_id` (the OBU header's `extended_layer_id`)
  to select the § 5.11 global-vs-local branch exactly as the parser does.
  - **Reject-before-write** (scratch-writer; never panics on a constructed model): byte-alignment; a
    `payloads` length that disagrees with `ops_cnt`; a per-payload / per-entry `index` that disagrees
    with its position; every gated `Option` whose presence disagrees with its gate (`priority` /
    `intent` / `mlayer_info_idc` / `local_reserved_2bits` present iff `ops_cnt > 0`, the
    global-vs-local choice, and the `intent_present` / `ptl_present` / `color_info_present` /
    `mlayer_info_idc` sub-struct gates); and field-width / `uvlc` rejects from the primitives.
- **Dispatch** (`write/dispatch.rs`): route `ParsedObu::OperatingPointSet` to the new writer + the
  generic tail (threading `obu_xlayer_id`) instead of `Unimplemented`; it carries no passthrough. Six
  types remain unwritten.
- **Error** (`write/error.rs`): add `WriteError::NonCanonicalOperatingPointSet { what }`.

## Validator impact

None.

## Non-goals

- No writers for the other six unwritten OBU types; no model change; no public `encode` command.

## Impact

- Crate: `crates/splot-core` (additive `write::operating_point_set` + one `WriteError` variant + the
  dispatch arm).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (the OPS write rows + `ENC-BITSTREAM-WRITER` note) +
  regenerated `docs/FEATURE-STATUS.md`.
