# Proposal: dispatch frame-carrying OBU payloads

## Feature IDs

- `AV2-5.2.1-OBU-DISPATCH` (the 11-type catch-all residual closes)
- `AV2-5.19-TILE-GROUP` / `AV2-5.18-FRAME-HEADER` (the mapped families'
  dispatch arms)

## Why

`dispatch_obu_payload` returns `Unimplemented` for the 11 frame-carrying
OBU types (CLK/OLK/Leading-/RegularTileGroup/Switch/RAS and the SEF/TIP/
Bridge family) even though the validator and inspector parse them via
direct calls with cross-OBU state. `inspect`'s payload status for frame
OBUs stays "unimplemented" for syntax the project parses — dishonest
surface.

## What Changes

1. Route the frame-carrying types through dispatch honestly. Design
   constraint: dispatch is stateless — it can parse exactly the
   state-free §5.18.2/§5.19 prefix (is_first_tile_group,
   frame_header_present_flag, cur_mfh_id, the direct seq reference, the
   per-type fixed fields like bridge_frame_ref_idx / SEF fields needing
   only the header... ground what is truly state-free per type) and
   must return an honest status for the state-dependent remainder (a
   new PayloadStatus variant carrying the parsed prefix + the reason
   the rest needs state, rather than blanket Unimplemented).
2. The inspector keeps its stateful parse as the richer surface; its
   payload status reflects the stateful outcome (already does via the
   frame-header views — reconcile so the dispatch-level status and the
   stateful views are consistent and documented).
3. The validator's direct-call path is unchanged (dispatch stays the
   stateless front door).
4. Matrix: the dispatch row's residual closes or narrows honestly.

## Non-goals

- Stateful parsing inside splot-core's dispatch.
- § 5.20 payload parsing.

## Acceptance criteria

- [ ] No frame-carrying type returns blanket Unimplemented from
  dispatch; per-type tests (prefix parsed, honest state-dependent
  status, EOF); inspect surface consistent; matrix proof;
  `cargo xtask ci` green.
