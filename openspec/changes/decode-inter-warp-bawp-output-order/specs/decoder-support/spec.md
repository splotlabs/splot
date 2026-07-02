## MODIFIED Requirements

### Requirement: First-inter-frame frontier warp, BAWP, and output-order subset
The decoder SHALL derive EXTENDWARP warp models per § 7.13.3.24 and
LOCALWARP models per § 7.12.3/§ 7.13.3.23, SHALL predict invalid-shear
or sub-8x8 warp geometry with the § 7.13.3.20 extended block warp, SHALL
apply § 7.13.3.25 block adaptive weighted prediction after motion
compensation, SHALL fill the MV stack from the § 7.12.2.21 reference MV
bank with the § 5.20.2.2 per-superblock reset, and SHALL output frames
in § 7.21 display order. TIP frames (`tip_frame_mode == 1`) SHALL be
rejected with a structured diagnostic after the bit-exact header read.

#### Scenario: Warp dependency chain parses through frame 2
- **GIVEN** the ac0ej3 mission stream
- **WHEN** decoding reaches coded frame 2
- **THEN** every EXTENDWARP/LOCALWARP/BAWP block decodes and the next
  defer is a later frame's feature gate, before any output

#### Scenario: Output frame 0 is unchanged
- **GIVEN** the ac0ej3 mission stream decoded with a one-frame limit
- **WHEN** the display-order scheduler releases frames
- **THEN** output frame 0 is byte-identical to the avmdec raw output
