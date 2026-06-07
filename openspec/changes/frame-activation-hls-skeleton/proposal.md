# Proposal: frame activation HLS skeleton

## Summary

Add a bounded AV2 frame-header and tile-group **prefix** parser that extracts only
the fields required for sequence-header activation and HLS availability checks.
Extend validator state so frame headers can reference sequence headers and
multi-frame headers, allowing exact checks for the validator gaps currently blocked
on `AV2-5.18-FRAME-HEADER`.

## Why

The current validator has implemented sequence-header parsing, content-interpretation
timing checks, temporal-unit ordering, and partial HLS availability. The remaining
exactness gaps are now blocked by frame-header syntax:

- the generic `hls/unavailable-sequence-header` diagnostic is reserved for the
  frame-header reference path;
- `cur_mfh_id` availability cannot be checked without frame-header prefix parsing;
- active sequence-header state and CVS-scoped fingerprints are currently approximated
  from OBU order;
- exact CLK/OLK activation requires the frame header that references and activates
  `seq_header_id`.

A full §5.18 implementation is much larger than needed for this milestone. A
prefix-only skeleton unlocks the next validator correctness layer without committing
to full frame/tile decoding.

## What changes

- Add a frame-header prefix parser for the activation/reference fields
  (`cur_mfh_id`, `seq_header_id_in_frame_header`).
- Add a tile-group prefix parser far enough to locate an optional frame header in
  §5.19 (`is_first_tile_group`, `frame_header_present_flag`).
- Extend the HLS availability store with multi-frame-header records so `cur_mfh_id`
  can be resolved.
- Emit generic HLS diagnostics for frame-header references.
- Move modeled sequence-header activation and CVS scoping from OBU-level
  approximations toward frame-header-driven state for the parsed CLK/OLK paths.
- Update the implementation matrix, diagnostics registry, feature status, and tests.

## Non-goals

- Full §5.18 frame-header parse.
- Full §5.19/§5.20 tile-group payload parse.
- Entropy/range decoding.
- Reference-frame update, random-access long-term reference validation, Annex A level
  checks, or Annex E decoder model.
- Bitstream writer or encoder work.

## Feature IDs

- `AV2-5.18-FRAME-HEADER`
- `AV2-5.18.1-FRAME-HEADER-GENERAL`
- `AV2-5.18.2-FRAME-HEADER-INFO`
- `AV2-5.19-TILE-GROUP`
- `AV2-5.7-MULTI-FRAME-HEADER`
- `AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT`
- `AV2-7.3.8-HLS-AVAILABILITY`

## Acceptance criteria

- A frame header with `cur_mfh_id == 0` that references a missing
  `seq_header_id_in_frame_header` emits `hls/unavailable-sequence-header`.
- A frame header with `cur_mfh_id > 0` that references a missing multi-frame header
  emits `hls/unavailable-multi-frame-header`.
- Multi-frame-header records are stored and consumed by frame-header reference checks.
- CLK/OLK paths parsed by the skeleton activate the referenced sequence header instead
  of relying only on sequence-header OBU order.
- Sequence-header fingerprints and content-interpretation records are scoped more
  exactly for frame-header-modeled CVS boundaries.
- Full frame-header / tile payload rows remain partial/todo, not incorrectly marked
  done.
- `cargo xtask ci` and `openspec validate frame-activation-hls-skeleton --strict`
  pass.
