## MODIFIED Requirements

### Requirement: First-inter-frame frontier warp, BAWP, and output-order subset
The decoder SHALL derive EXTENDWARP warp models per § 7.13.3.24 and
LOCALWARP models per § 7.12.3/§ 7.13.3.23, SHALL predict invalid-shear
or sub-8x8 warp geometry with the § 7.13.3.20 extended block warp using
the per-8x8 reference bounding box, SHALL apply § 7.13.3.25 block
adaptive weighted prediction after motion compensation (skipping the
§ 5.20.7.15 `inter_intra` read that `use_bawp` disables), SHALL fill
the MV stack from the § 7.12.2.21 reference MV bank — contents cleared
once per superblock row, hit counters reset and re-seeded per
superblock per § 5.20.2.2, the unit budget accrued for non-inter blocks
per § 5.20.7 `update_ref_mv_count`, and one `PruneCount` budget shared
across the spatial scan, bank fill, and § 7.12.2.20 global-MV dedup —
and SHALL output frames in § 7.21 display order with the § 7.23
per-slot evict-then-store interleave and § 5.18.2 extended order hints
at the scheduling surface (a frame whose extended hint diverges from
its coded LSB defers fail-closed while parse-side consumers remain
LSB-windowed). TIP frames (`tip_frame_mode == 1`) SHALL be rejected
with a structured diagnostic after the bit-exact header read.

#### Scenario: Warp dependency chain parses through frame 2
- **GIVEN** the local decoder mission stream
- **WHEN** decoding reaches coded frame 2
- **THEN** every EXTENDWARP/LOCALWARP/BAWP block decodes and the next
  defer is a later frame's feature gate, before any output

#### Scenario: Output frame 0 is unchanged
- **GIVEN** the local decoder mission stream decoded with a one-frame limit
- **WHEN** the display-order scheduler releases frames
- **THEN** output frame 0 is byte-identical to the avmdec raw output
