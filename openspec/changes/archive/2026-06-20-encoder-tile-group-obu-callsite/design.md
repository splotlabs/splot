## Context

Brick 3 (the keystone) produces the `(FrameHeaderCore, CoreSeqView)` pair; bricks 1–2 added
the `TileGroupStructure::single_tile_first_group` / `TileGroupFraming::single_tile`
constructors. `write_tile_group_obu(writer, core, seq, mfh, first_pic, structure, framing,
tile_data, is_first_tile_group)` is the § 5.19 / § 5.20.1 first-tile-group writer. This
brick composes them.

## Goals / Non-Goals

- Goal: the first encoder writer-input end-point that emits a `tile_group_obu()` payload
  from coded tile bytes.
- Non-Goals: the § 5.2.2 OBU header / size wrapper, a complete spec-conformant coded tile
  (the `tile_data` content is the caller's, a separate axis tracked by the block-symbol
  trace), multi-tile, inter, a temporal unit, a packet, or `receive_packet` output.

## Decision: tile_data is a parameter, the header is fixed

The frozen-tier `(core, seq)` are built internally (the keystone's self-contained design),
but `tile_data` is a genuine parameter — different frames carry different coded tile bytes,
and the § 5.20.1 framing is derived from `tile_data.len()`. So `fn(tile_data) -> payload` is
the right shape: the fixed context is internal, the variable content is the input. An empty
slice is a § 8.2.2 zero-size-tile defect `TileGroupFraming::single_tile(0)` returns and
`write_tile_group_obu` rejects, surfaced as a typed `Write` error (no panic).

## Oracle

The writer's own `whole_obu_round_trips_stage_by_stage` test already proves
`write_tile_group_obu` round-trips against a test core; this brick proves the **keystone**
feeds it. The round-trip test reparses the payload's tile-group prefix
(`is_first_tile_group`, `frame_header_present_flag`, the embedded frame header) and asserts
the coded tile bytes are the byte-aligned trailing region of the payload (the lone last
tile reads no size field and takes the remainder). The reject test pins the
empty-`tile_data` error.

## Error model

`MinimalIntraTileGroupError` wraps `MinimalIntraCoreError` (`Core`, from the assembler) and
`WriteError` (`Write`, from the tile-group writer) via `#[from]`, so `?` propagates both.
