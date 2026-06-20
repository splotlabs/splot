## Context

The decoder minimal-tier IVF frame requires its OBUs in order `OBU_TEMPORAL_DELIMITER`,
`OBU_SEQUENCE_HEADER`, then the frame OBU. All three are now produced as Annex B OBUs, and
the IVF writers (`write_ivf_header` / `write_ivf_frame`, `io::Write`-based) exist.

## Decision: concatenate the Annex B OBUs, wrap in one IVF frame

The AV2 temporal unit is the concatenation of the three length-delimited Annex B OBUs; the
IVF frame payload is that temporal unit. The IVF header is `AV02` 64x64 with timebase 30/1
and one frame (matching the committed conformance vector's header).

## Consistency

For the temporal unit to be coherent, the frame header must parse against the sequence
header. The frame header was built (in the keystone) against
`new_minimal_intra_single_picture(64, 64)` with the `Block64x64` override; `from_sequence`
of this sequence header yields the same tier on every field that drives frame-header parsing
— `single_picture_header_flag`, `OrderHintBits = 0`, `NumRefFrames = 2`, the SCC `SELECT`
force fields, 64x64 maxima, `(enable_avg_cdf, avg_cdf_type) = (true, 1)`,
`monotonic_output_order_flag`, and `seq_sb_size = Block64x64`. The test pins the load-bearing
fields. (The `Block64x64` override added for the frame-header keystone's [Important] review
finding is what makes this consistent — the default `Block128x128` would not be.)

## Non-Goals

A decode-hash match to the conformance vector: `tile_data` is a caller input, so a complete
spec-conformant coded tile (and `base_q_idx`) is a separate axis — the block-symbol trace.
The result is a structurally valid IVF with consistent headers, not yet a decodable image.
A packet / `receive_packet` output, CLI success, or Baseline Encoder Profile v1.

## Error model

`MinimalIntraIvfError` wraps the three OBU assemblers' errors and `std::io::Error` from the
IVF writers via `#[from]`, so `?` propagates each.
