## Context

Brick 2 proved the general-intra DC skip-block trace round-trips through one § 8.2 coder. The
remaining question for emitting a decodable tile was the exact `tile_data` byte layout: does
the general decode path read `tile_data` as pure finalized symbol bytes, or is there a
structural prefix?

## Decision: tile_data is pure §8.2.4-finalized symbol bytes

A multi-agent verification confirmed, against the decoder source, that for a single last tile
the `tile_data` is pure § 8.2.4-finalized symbol-coder bytes with **no structural prefix**:

- The tile-group framing parser yields `tile_data_offset = pos` with no `tile_size_minus_1`
  field for the last (here only) tile; the decoder slices `[offset..+size]` and hands it
  unmodified to the § 8.2 `SymbolDecoder`, which reads from byte 0.
- `SymbolEncoder::finish()` (§ 8.2.4 `exit_symbol`) emits a pure, byte-aligned, MSB-first
  `Vec<u8>` validated against the exit window, with no CDF metadata.
- Encoder and the roundtrip decoder both default to `CdfUpdateMode::Enabled`.

Therefore `encode_block_symbol_trace(compose_general_intra_dc_skip_block_trace())` is directly
usable as `tile_data`. Brick 3 exposes exactly that and re-asserts the decodability through
the existing roundtrip.

## The muxing contract for a later brick

The verification surfaced one conditional constraint: the real tile reader derives its
`CdfUpdateMode` from the frame's `disable_cdf_update`. Because this trace is coded under
`CdfUpdateMode::Enabled`, the muxing frame header must set `disable_cdf_update == 0` (the
existing minimal header does). It must also set `base_q_idx <= 90` so the decoder derives
coefficient CDF q-context `0` — the q-context brick 2's `txb_skip` rows are coded under. The
existing `build_minimal_intra_clk_core` hardcodes `base_q_idx == 255` (q-context 3), so a
`base_q_idx <= 90` frame-header variant (ideally matching the AVM-validated q80 fixture's
`base_q_idx == 80`) is a separate splot-core brick. The cross-crate decode oracle belongs in
splot-cli (the only crate that depends on both splot-encode and splot-decode).

## Scope discipline

Brick 3 is intentionally thin and fully provable inside splot-encode (no cross-crate
dependency). It does not assemble a tile-group OBU, a frame, an IVF, or claim a decode — those
are the next bricks, with distinct blast radii.
