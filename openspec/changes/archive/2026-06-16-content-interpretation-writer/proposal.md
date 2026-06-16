# Change: content-interpretation-writer

## Feature IDs

- `AV2-5.15-CONTENT-INTERPRETATION` (write: `todo` → `done`)
- `ENC-BITSTREAM-WRITER` (the fourth of the unwritten OBU-type body writers)

## Why

Continue moving the parser-modeled OBU types from `Unimplemented` to round-trippable in the
complete-OBU dispatch. `content_interpretation_obu()` (§ 5.15) is the next target: `ci_scan_type_idc`
plus four gated `Option` sub-structs (color description, chroma sample position, aspect-ratio info,
timing info) and a tolerated `ci_reserved_2bit`. No delta coding, no derived stored fields (the
`derived_color` / `derived_sample_aspect_ratio` are query methods, not wire fields).

## What changes

- **Writer** (`crates/splot-core/src/write/content_interpretation.rs`, new; additive, no model change):
  `write_content_interpretation(writer, ci)` — the inverse of `parse_content_interpretation`
  (§ 5.15), including a private `write_timing_info` inverting the shared `parse_timing_info`
  (`headers/sequence.rs`), field order preserved.
  - **Reproduce-verbatim, not reject** for the parser-tolerated values: `ci_reserved_2bit` (§ 6.14
    wants 0 but the parser preserves any value), a reserved `ci_color_description_idc` (6..=127), a
    reserved `ci_aspect_ratio_idc` (17..=254), and a reserved `ScanTypeIdc` — the writer reproduces
    them faithfully, because once this OBU is writable the `roundtrip_obu_bytes` fuzz target
    round-trips every parsed model and an over-rejection of a parser-producible value would panic it.
  - **Reject-before-write** (scratch-writer; never panics) only for the strictly-decidable structural
    inconsistencies: an `extended_sar` presence that disagrees with `ci_aspect_ratio_idc == 255`, a
    color-`primaries` presence that disagrees with `ci_color_description_idc == 0`, byte-alignment,
    and field-width rejects from the primitives.
- **Dispatch** (`write/dispatch.rs`): route `ParsedObu::ContentInterpretation` to the new writer + the
  generic tail instead of `Unimplemented`; it carries no passthrough. Five types remain unwritten.
- **Error** (`write/error.rs`): add `WriteError::NonCanonicalContentInterpretation { what }`.

## Validator impact

None.

## Non-goals

- No writers for the other five unwritten OBU types; no model change; no public `encode` command.

## Impact

- Crate: `crates/splot-core` (additive `write::content_interpretation` + one `WriteError` variant +
  the dispatch arm).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (`AV2-5.15-CONTENT-INTERPRETATION` write `done` +
  `ENC-BITSTREAM-WRITER` note) + regenerated `docs/FEATURE-STATUS.md`.
