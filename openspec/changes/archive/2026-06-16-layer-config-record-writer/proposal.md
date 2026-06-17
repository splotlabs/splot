# Change: layer-config-record-writer

## Feature IDs

- `AV2-5.8-LAYER-CONFIG-RECORD` (write: `todo` → `done`)
- `ENC-BITSTREAM-WRITER` (the seventh of the unwritten OBU-type body writers; one remains after it)

## Why

Continue moving the parser-modeled OBU types from `Unimplemented` to round-trippable in the
complete-OBU dispatch. `layer_config_record_obu()` (§ 5.8) is the next target and the largest by
far: a global-vs-local branch (selected by `obu_xlayer_id`) over a deeply-nested structure —
`lcr_global_info` / `lcr_local_info`, the length-bounded `lcr_global_payload` (whose trailing
`lcr_remaining_payload_bit` filler is derived from the declared `lcr_data_size`), `lcr_xlayer_info`
with its `byte_alignment()` and mutually-exclusive embedded-layer-vs-atlas else-branch, the per-set-bit
`lcr_embedded_layer_info` with its own per-iteration `byte_alignment()`, and the `lcr_aggregate_info`,
`lcr_seq_profile_tier_level_info`, `lcr_rep_info`, and `lcr_xlayer_color_info` leaf structures. After
it lands, only `QuantizationMatrix` (§ 5.13) remains unwritten.

## What changes

- **Writer** (`crates/splot-core/src/write/layer_config_record.rs`, new; additive, no model change):
  `write_layer_config_record(writer, record)` — the inverse of `parse_layer_config_record` and the
  § 5.8.1–5.8.9 sub-struct parsers, field order preserved, drafted into a scratch and appended only on
  full success.
  - **Reject-before-write** (scratch-writer; never panics): every gated `Option` presence vs its
    gate (`lcr_global_atlas_id_present_flag` vs `global_atlas_id`, the atlas-present `reserved_zero_3bits
    == 0` rule, `lcr_aggregate_info_present_flag` vs `aggregate_info`, the four `lcr_xlayer_info`
    present flags vs their `Option`s, the mutually-exclusive embedded-layer-vs-atlas else-branch, the
    `lcr_format_info`/`lcr_cropping_window`/`lcr_*_atlas` gates, the `layer_color_description_idc == 0`
    primaries gate, the per-embedded-layer atlas/aux/view/dependent/expected-resolution gates); every
    count-vs-derived consistency (`seq_ptl_infos` / `payloads` against the `lcr_xlayer_map` set-bit
    ids; `lcr_embedded_layer_info.layers` against the `lcr_mlayer_map` set bits); the
    `lcr_global_payload` filler invariant (`content_bits + remaining_payload_bits == lcr_data_size *
    8`); and field-width / `uvlc` / `rg` / `f(n)` domain rejects.
  - **Reproduce-verbatim** the parser-tolerated values within their descriptor domain (the
    `lcr_*_reserved_zero_*` bits — § 6.8 says they "shall be equal to 0" but the parser retains them —
    `lcr_global_config_record_id`, `lcr_local_id`, the aggregate/PTL/color descriptive fields) so a
    parsed model always round-trips.
- **Dispatch** (`write/dispatch.rs`): route `ParsedObu::LayerConfigurationRecord` to the new writer +
  the generic extensible tail instead of `Unimplemented`; it carries no passthrough. One type remains
  unwritten (`QuantizationMatrix`).
- **Error** (`write/error.rs`): add `WriteError::NonCanonicalLayerConfigRecord { what }`.

## Validator impact

None.

## Non-goals

- No writer for `QuantizationMatrix`; no model change; no public `encode` command.

## Impact

- Crate: `crates/splot-core` (additive `write::layer_config_record` + one `WriteError` variant + the
  dispatch arm).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (the `AV2-5.8-LAYER-CONFIG-RECORD` write rows +
  `ENC-BITSTREAM-WRITER` note) + regenerated `docs/FEATURE-STATUS.md`.
