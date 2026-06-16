# Change: tile-group-obu-composer

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.19-TILE-GROUP` (the composing `tile_group_obu()` writer for the first tile group;
  advances the umbrella `write` coverage — the row stays `partial`)

## Why

The third and final tile-group slice (after `tile-group-structure-writer` and
`tile-group-payload-writer`): the composing `write_tile_group_obu` that emits a whole **first**
intra `tile_group_obu()` payload — the prefix, the embedded `frame_header()`, the § 5.19 structure,
and the § 5.20.1 payload framing — in one byte sequence. With it `splot` can serialize a complete
intra tile-group OBU payload from the model, the foundation for the container muxers and the
writer → `splot validate` cross-tool tests.

## What changes

- **Composer** (`crates/splot-core/src/write/tile_group.rs`): `write_tile_group_obu` (first-tile-group
  form), the inverse of `parse_tile_group_prefix` + `frame_header()` + `parse_tile_group_structure` +
  `parse_tile_group_framing` for `is_first_tile_group == 1`. It emits, in § 5.19 read order:
  1. `is_first_tile_group` `f(1)` = `1` (the first-group form; `frame_header_present_flag` is inferred
     `1`, so no bit).
  2. `frame_header()` via the existing [`write_frame_header_core`] (the intra path; it takes the
     `FrameHeaderCore` + `CoreSeqView` + optional `MfhFrameView` + `first_picture_in_tu`) — **option A**
     from the scoping audit: the composer takes the already-built frame-header model and delegates,
     rather than re-deriving it.
  3. the § 5.19 structure via `write_tile_group_structure`.
  4. the § 5.20.1 payload framing via `write_tile_group_payload`.
  The whole composition is drafted into a scratch `BitWriter` and committed (`append`) only on full
  success, so a sub-writer reject leaves the caller's writer untouched (reject-before-write for the
  whole OBU payload; `bit_len()` unchanged).
- **Reject-before-write**: a non-first form (`is_first_tile_group == 0`, the `frame_header_copy()`
  continuation) is out of scope and rejected with `WriteError::NonCanonicalTileGroup`
  (`"continuation_unsupported"`); plus every reject the delegated sub-writers raise.
- **No model change.** Purely additive; reuses the existing frame-header / structure / payload
  writers and the `NonCanonicalTileGroup` error.

## Validator impact

None. No new diagnostics.

## Non-goals

- No `frame_header_copy()` continuation (the `is_first_tile_group == 0` tile group — a follow-up).
- No OBU header / size / trailing-bits framing (the OBU writer's job), no inter / BRU / bridge paths,
  no `decode_tile()` block syntax.
- No public `encode` command.

## Impact

- Crate: `crates/splot-core` (additive `write` composer; reuses existing writers + `NonCanonicalTileGroup`).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (the `AV2-5.19-TILE-GROUP` row WRITER note) + regenerated
  `docs/FEATURE-STATUS.md`.
