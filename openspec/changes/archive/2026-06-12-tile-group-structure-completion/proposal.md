# Proposal: complete the § 5.19 tile-group structure on intra paths

## Feature IDs

- `AV2-5.19-TILE-GROUP` (the post-frame-header remainder)
- `AV2-5.18.2-FRAME-HEADER-INFO` (the first tile group's header/payload
  boundary becomes exact on intra streams)

## Why

`parse_tile_group_prefix` stops after the optional `frame_header()`. The
§ 5.19 remainder (mirror `05-syntax-structures.md`:8431-8530) is now
decidable on intra streams: `NumTiles = TileCols * TileRows` and
`TileColsLog2/TileRowsLog2` come from the parsed intra `tile_info()`
(PR #59 completes the intra header), so `tile_start_and_end_present_flag`,
`tg_start`/`tg_end` (f(tileBits)), `byte_alignment()`, and the
`headerBytes`/`sz` handoff into `tile_group_payload(sz)` can be parsed and
validated. `bru_inactive`/`use_bru` come from the unparsed § 5.18.2 inter
region — the BRU arms must stop explicitly until
frame-header-inter-reference-paths lands. This also gives the FIRST tile
group an exact header/tile-data boundary (closing the remaining PR #59
padding-ambiguity scenario for intra streams) and the § 5.18.1
`bru_inactive` trailing-bits arm its named home.

## What Changes

1. Parse the § 5.19 remainder after the frame header on intra-complete
   paths: `tile_start_and_end_present_flag` (NumTiles > 1 gate),
   `tg_start`/`tg_end`, `byte_alignment()`, and record the
   `headerBytes`/remaining-`sz` payload boundary (the payload itself stays
   unparsed — § 5.20 is its own change; named residual).
2. tg-range validation: ground the governing § 6.18/§ 6.x semantics for
   `tg_start`/`tg_end` (find the exact conformance clauses — e.g.
   tg_start of the first tile group of a frame, continuity across tile
   groups, tg_end >= tg_start, bounds vs NumTiles) and add the
   locally-decidable diagnostics with citations; under-report whatever
   needs unmodeled state.
3. BRU arms: frames whose `use_bru`/`bru_inactive` cannot be derived
   (inter region unparsed) stop honestly; intra-complete frames derive
   them per the § 5.18.2 intra inferences (find what the intra path
   implies for use_bru/bru_inactive — ground it; if they are
   intra-inferred constants the arms are decidable now).
4. EOF in the new region preserves facts per the established pattern;
   truncation in the fully-modeled region surfaces per the PR #59
   precedent.
5. `inspect` surfaces the tile-group structure (tg range, payload size).

## Non-goals

- § 5.20 tile_group_payload parsing (item 26's scope).
- Inter-path BRU semantics (frame-header-inter-reference-paths).

## Acceptance criteria

- [ ] Intra streams parse the full § 5.19 structure; tg-range
  diagnostics with citations; positive/negative/EOF per element; BRU
  honest stops tested; matrix proof; `cargo xtask ci` green.
