# Proposal: validate § 5.20 tile-payload framing boundaries

## Feature IDs

- `AV2-5.20-TILE-GROUP-PAYLOAD` (todo → the § 5.20.1 framing slice)
- `AV2-5.19-TILE-GROUP` (the payload handoff consumes the recorded
  boundary)

## Why

Nothing of § 5.20 exists. The boundary slice is decidable without
decoding blocks: § 5.20.1 (mirror `05-syntax-structures.md`:8549-8640)
frames each non-last tile with `tile_size_minus_1 le(TileSizeBytes)`
(`TileSizeBytes` from the parsed `tile_info()`), bookkeeps
`sz -= tileSize + TileSizeBytes`, and gives the last tile the remaining
`sz`; bridges skip the size fields entirely. Truncated/overflowing tile
sizes are real bitstream defects the validator can prove from the
framing alone. `decode_tile()` and § 5.20.2-.10 stay explicit child
territory ("split further before pixel-reconstruction-dependent
checks").

## What Changes

1. Parse the § 5.20.1 framing for tile groups whose § 5.19 structure
   completed (PR #61's `headerBytes`/payload boundary + tg range +
   `TileSizeBytes` from tile_info): per-tile `tile_size_minus_1`,
   bookkeeping, lastTile/IsBridge arms — exactly per the mirror.
2. Diagnostics for the provable framing defects (a tile size whose
   `tileSize + TileSizeBytes` exceeds the remaining `sz`; a last tile
   with nonpositive remaining `sz` where tiles remain — ground each
   condition in § 5.20.1/§ 6.x and cite). Ground what § 8's
   `init_symbol(tileSize)`/`exit_symbol()` make checkable WITHOUT
   symbol decoding (e.g. minimum tile size for the arithmetic init —
   read § 8.2; if nothing is checkable without decoding, name the
   residual).
3. The per-tile byte ranges are recorded/surfaced (`inspect` shows the
   tile framing); BRU tile arms stop honestly where `use_bru` state is
   inter-gated (named).
4. Unknown routing: incomplete § 5.19 structure, ambiguous boundaries,
   unparsed tile_info → no framing judgments.

## Non-goals

- `decode_tile()` / block syntax / symbol decoding (§ 5.20.2-.10 child
  rows; § 8 entropy decode; § 9 table consumers).
- Pixel reconstruction.

## Acceptance criteria

- [ ] Conformant multi-tile framing parses and is surfaced; each
  framing defect fires with citations; positive/negative/EOF per arm;
  Unknown routing tested; matrix proof; `cargo xtask ci` green.
