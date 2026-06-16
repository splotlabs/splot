# Change: msdo-writer

## Feature IDs

- `AV2-5.6-MSDO` (write: `todo` → `done`)
- `ENC-BITSTREAM-WRITER` (the second of the eight remaining unwritten OBU-type body writers)

## Why

Continue moving the parser-modeled OBU types from `Unimplemented` to round-trippable in the
complete-OBU dispatch. `multistream_decoder_operation_obu()` (§ 5.6) is the next-most-tractable: flat
fixed-width fields plus one bounded per-substream loop, no deeply-nested syntax.

## What changes

- **Writer** (`crates/splot-core/src/write/msdo.rs`, new; additive, no model change):
  `write_msdo(writer, msdo: &MultistreamDecoderOperation)` — the inverse of `parse_msdo` (§ 5.6):
  `num_streams_minus_2` `f(3)`, `multistream_profile_idc` `f(5)`, `multistream_level_idx` `f(5)`,
  `multistream_tier` `f(1)`, `multistream_even_allocation_flag` `f(1)`, optional
  `multistream_large_picture_idc` `f(3)` (present iff allocation is not even), then
  `num_streams_minus_2 + 2` per-substream entries (`sub_xlayer_id` `f(5)`, `sub_stream_max_profile`
  `f(5)`, `sub_stream_max_level` `f(5)`, `sub_stream_max_tier` `f(1)`), and
  `multistream_doh_constraint_flag` `f(1)`.
  - **Reject-before-write** (scratch-writer; never panics on a constructed model): byte-alignment;
    a `multistream_large_picture_idc` presence that disagrees with `multistream_even_allocation_flag`
    (the gated-field rule); a `sub_stream_count` that disagrees with `num_streams_minus_2 + 2`; a
    non-zero unused `sub_streams[count..]` slot (the parser leaves those zero, so a non-zero slot is
    parser-unproducible and would not round-trip); and field-width rejects from the primitives.
- **Dispatch** (`write/dispatch.rs`): route `ParsedObu::Msdo` to the new writer + the generic tail
  (MSDO is not extensible) instead of `Unimplemented`; it carries no passthrough. Seven types remain.
- **Error** (`write/error.rs`): add `WriteError::NonCanonicalMsdo { what }` (the per-family pattern).

## Validator impact

None.

## Non-goals

- No writers for the other seven unwritten OBU types; no model change; no public `encode` command.

## Impact

- Crate: `crates/splot-core` (additive `write::msdo` + one `WriteError` variant + the dispatch arm).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (`AV2-5.6-MSDO` write `done` + `ENC-BITSTREAM-WRITER` note) +
  regenerated `docs/FEATURE-STATUS.md`.
