# Change: tile-group-continuation-writer

## Feature IDs

- `AV2-5.19-TILE-GROUP` (write: `partial` → `partial`, completing the OBU composer)
- `ENC-BITSTREAM-WRITER` (the deferred non-first `frame_header_copy()` tile-group continuation composer)

## Why

The first-tile-group `tile_group_obu()` composer (`write_tile_group_obu`, `is_first_tile_group == 1`)
landed earlier and explicitly rejects the non-first form with `what == "continuation_unsupported"`. A
coded frame with more than one tile group emits the remaining groups as **continuations**
(`is_first_tile_group == 0`): each carries an explicit `frame_header_present_flag` and, when set, a
`frame_header_copy()` — a verbatim bit-copy of the first group's `frame_header()` (§ 5.18.1) — then the
same § 5.19 structure and § 5.20.1 payload framing. This change adds the continuation composer so a
multi-tile-group coded frame round-trips end-to-end.

## What changes

- **Parser accessor** (`crates/splot-core/src/headers/tile_group.rs`, additive — read-only data
  exposure, no parse-behavior change): expose the recorded first-header copy bits so the writer can
  re-emit `frame_header_copy()` verbatim — make `RecordedFrameHeaderBits::bit` public (it already
  returns the MSB-first bit at an offset, used by the § 6.17.1 copy check).
- **Writer** (`crates/splot-core/src/write/tile_group.rs`, additive): `write_tile_group_continuation_obu`
  — the inverse of `parse_tile_group_prefix` (`is_first_tile_group == 0`), the `frame_header_copy()`
  region, `parse_tile_group_structure`, and `parse_tile_group_framing`. In § 5.19 read order it emits
  `is_first_tile_group = 0` `f(1)`, `frame_header_present_flag` `f(1)`, then — when the flag is set —
  the recorded first header's `NumFrameHeaderBits` copy bits verbatim, then delegates to the existing
  `write_tile_group_structure` (which, unlike the first-group composer, does **not** require
  `tg_start == 0`) and `write_tile_group_payload`. Drafted into a scratch and committed only on full
  success.
  - **Reject-before-write** (never panics): a non-byte-aligned writer; a `frame_header_present_flag`
    that disagrees with whether recorded copy bits are supplied; plus every reject the delegated
    structure / payload sub-writers already raise (degenerate / out-of-range structure, defective /
    mismatched framing).
- **No model change**; the continuation composer takes the recorded first-header bits, the
  `frame_header_present_flag`, the shared `TileGroupLayout` / `TileSizeBytes` (from the first group's
  `tile_info()`), and the structure / framing / tile-data as inputs (the parser never produces a single
  non-first-tile-group struct — the pieces are parsed and validated separately).

## Validator impact

None.

## Non-goals

- No re-derivation of the copy bits from a `FrameHeaderCore` (the continuation is a *bit-copy*, so the
  bits come from the recorded first header, not a re-serialized core); no public `encode` command.

## Impact

- Crate: `crates/splot-core` (one additive parser accessor + `write::tile_group::write_tile_group_continuation_obu`).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (the `AV2-5.19-TILE-GROUP` writer note + `ENC-BITSTREAM-WRITER`
  note) + regenerated `docs/FEATURE-STATUS.md` if a status field changes.
