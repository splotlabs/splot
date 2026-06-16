# Change: atlas-segment-writer

## Feature IDs

- `AV2-5.9-ATLAS-SEGMENT` and `AV2-5.9.1-ATLAS-LABEL-SEGMENT-INFO` (write: `todo` → `done`)
- `ENC-BITSTREAM-WRITER` (the sixth of the unwritten OBU-type body writers)

## Why

Continue moving the parser-modeled OBU types from `Unimplemented` to round-trippable in the
complete-OBU dispatch. `atlas_segment_info_obu()` (§ 5.9) is the next target: flat fixed-width and
`uvlc` fields (no delta coding or bit-width canonicalization), but a deeply-gated structure — an
`ats_atlas_segment_mode_idc` selecting one of five `AtlasModeInfo` bodies (single / enhanced / basic /
multistream / multistream-with-alpha), a `num_segments` derived from the mode, the § 5.9.1 label
segment-id assignment, and the region/uniform sub-structures with their gated `Option`s and `Vec`
lengths.

## What changes

- **Writer** (`crates/splot-core/src/write/atlas_segment.rs`, new; additive, no model change):
  `write_atlas_segment(writer, atlas)` — the inverse of `parse_atlas_segment` + the § 5.9.1–5.9.5
  sub-struct parsers, field order preserved.
  - **Reject-before-write** (scratch-writer; never panics): a `mode_info` variant that disagrees with
    the `mode`; a `num_segments` that disagrees with the value re-derived from the mode; every gated
    `Option` presence vs its gate (uniform vs explicit region dims, the § 5.9.1
    `ats_signaled_atlas_segment_ids_flag`); every count-vs-`Vec`-length and per-element index; and
    field-width / `uvlc` rejects.
  - **Reproduce-verbatim** the parser-tolerated, descriptive values (§ 6.9.2 states the atlas
    segment-id assignment elements carry no bitstream-conformance requirement) so a parsed model
    always round-trips.
- **Dispatch** (`write/dispatch.rs`): route `ParsedObu::AtlasSegment` to the new writer + the generic
  tail instead of `Unimplemented`; it carries no passthrough. Three types remain unwritten.
- **Error** (`write/error.rs`): add `WriteError::NonCanonicalAtlasSegment { what }`.

## Validator impact

None.

## Non-goals

- No writers for the other three unwritten OBU types; no model change; no public `encode` command.

## Impact

- Crate: `crates/splot-core` (additive `write::atlas_segment` + one `WriteError` variant + the
  dispatch arm).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (the AtlasSegment write rows + `ENC-BITSTREAM-WRITER` note) +
  regenerated `docs/FEATURE-STATUS.md`.
