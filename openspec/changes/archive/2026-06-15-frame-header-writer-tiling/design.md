# Design: frame-header-writer-tiling

## Context

`tile_info()` (§ 5.18.7.2) has three layout paths: reuse (`reuse_tile_params()`, § 5.18.7.4,
no layout bits), explicit (`tile_params()`, § 5.18.7.3), and bridge (zero `tile_params()`
bits, inferred uniform). It then optionally reads a `context_update_tile_id` /
`tile_size_bytes_minus_1` tail. The shared § 5.18.7.3 `tile_params()` writer already exists
(`write_tile_params`, landed with the sequence tile config).

## Decisions

- **Maintainer-approved model extension (`TileInfo.tile_params`).** The explicit branch's
  `parse_tile_layout` produces a full `TileParams` whose `uniform_spacing` flag drives the
  § 5.18.7.3 bit replay, but `TileInfo` discarded it (and the flag is not recoverable from
  `MiColStarts` / `MiRowStarts`). Surface it as `Option<TileParams>` (the same full-byte-exact
  exception taken for the #4b frame-config bits). The reuse/bridge branches write no
  `tile_params()` bits, so they leave it `None`.
- **Reuse `write_tile_params`; validate the reuse branch by re-derivation.** The explicit
  branch reuses the now-`pub(crate)` `write_tile_params`. The reuse branch re-derives the
  layout via `crate::tile::reuse_tile_params()` (exactly as the parser's reuse arm does — the
  uniform arm passes empty sequence start slices, the non-uniform arm passes the recorded
  `SeqSb*Starts`) and rejects a stored `TileInfo` that disagrees.
- **`sbShift2` is branch-dependent at the frame call site.** `MiColStarts[i] = sbColStarts[i]
  << sbShift2`, but `sbShift2` is `Mi_Width_Log2[seqSbSize]` on the uniform arm and
  `Mi_Width_Log2[SbSize]` on the non-uniform / reuse arm — and at the frame call site
  `seqSbSize` and `SbSize` can differ (unlike the sequence call site). The writer recovers
  `sbColStarts` / `sbRowStarts` from the stored `MiColStarts` / `MiRowStarts` with the
  per-branch shift.
- **Validate the explicit branch without misusing the sequence checker.**
  `check_tile_params_encodable` assumes `seqSbSize == SbSize` (true only at the sequence call
  site), so the frame writer instead forward-replays `write_tile_params` to a scratch
  `BitWriter`, reparses with `parse_tile_layout`, and compares — keeping the real writer
  untouched (reject-before-write) and correct for any `seqSbSize` / `SbSize` pairing.
- **Reject-before-write up front.** `check_tile_info_encodable` runs first and validates the
  reserved-level case, the `reuse_tile_info` inference, the reuse / explicit / bridge layout,
  and the tail (the `context_update_tile_id` gate + width, the `tile_size_bytes` presence /
  `1..=4` range), so every reject leaves `bit_len() == 0`.

## Testing

Round-trip via the public parser across reuse-uniform (signaled + inferred), reuse-non-uniform,
explicit-uniform (single / multi-tile), explicit-non-uniform, multi-tile with / without the
avg-CDF gate, bridge, and the not-eligible inferred-reuse-0 case. One reject test per
`WriteError` path (asserting `bit_len() == 0`). A round-trip property test driving
`parse_tile_info` on random bits + gating then re-emitting.
