## ADDED Requirements

### Requirement: General intra § 7.18 chroma CDEF admission
The decoder SHALL apply the AV2 § 7.18.1 chroma CDEF steps (steps 9-14) over the
two 4:2:0 chroma planes of an admitted general intra CDEF-active frame whose
strength set carries NONZERO chroma strengths (`cdef_uv_pri_strength` and/or
`cdef_uv_sec_strength`), in the same verified subset as the luma path
(8-bit 4:2:0, `CdefStrengths == 1`, `cdef_on_skip_txfm_frame_enable == 1`, single
tile, segmentation disabled). Per 8x8 unit the chroma pass SHALL derive the chroma
primary / secondary strengths from `cdef_uv_*_strength`, the chroma direction
`uvDir = (uv_pri == 0) ? 0 : Cdef_Uv_Dir[SubsamplingX][SubsamplingY][yDir]` (the
luma `yDir` from the § 7.18.2 direction search), and the chroma damping
`CdefDamping + coeffShift - 1`, then over each output sample of the subsampled 4x4
chroma block (`subX == subY == 1`) fetch the six § 7.18.3 directional taps via
`cdef_get_at` and apply the § 7.18.3 constrain / tap filter, writing the deringed
chroma sample back. The route gate SHALL no longer reject a CDEF-active frame for
nonzero chroma strengths; it SHALL still require `CdefStrengths == 1`,
`cdef_on_skip_txfm_frame_enable == 1`, a present damping / strength set, and 8-bit.

This requirement SHALL NOT claim a multi-strength (`CdefStrengths > 1`) frame, a
10-bit CDEF-active frame, non-4:2:0 chroma subsampling, multiple tiles, the other
in-loop filters, or inter frames; those remain rejected with a structured
`decode/unsupported-feature` diagnostic before any caller-visible output. A
CDEF-active frame whose chroma strengths are zero SHALL remain a chroma no-op
(byte-identical), and a CDEF-off frame SHALL still skip the pass entirely.

#### Scenario: nonzero-uv CDEF intra frame decodes to the oracle
- **WHEN** `splot decode` is given the committed
  `syn-2sb-cdefuv-intra-128x64-q170.ivf` — an 8-bit 4:2:0 intra key frame with two
  64x64 PARTITION_NONE superblocks (both DC_PRED luma; left non-follow H_PRED
  chroma, right DC chroma), `cdef_frame_enable == 1`, `CdefStrengths == 1`,
  `cdef_on_skip_txfm_frame_enable == 1`, `CdefDamping 5`, and a single strength set
  with `y_pri 10` / `y_sec 4` AND nonzero `uv_pri 2` / `uv_sec 4`
- **THEN** the general intra path reconstructs the frame, applies the § 7.18 CDEF
  pass in place (luma AND chroma), and succeeds
- **AND** the `--output-format raw` bytes equal the avmdec and dav2d raw outputs
  exactly (raw md5 `d783f353078cf156ba23dcfd3b2b50ad`)
- **AND** the § 7.18.1 chroma steps change over a thousand U and V samples each
  versus the same frame decoded with the chroma strengths forced to zero (the luma
  output is unchanged by the chroma strengths)

#### Scenario: chroma CDEF leaf path is unit-pinned
- **WHEN** the § 7.18 orchestration runs over a synthetic chroma ripple with a
  nonzero (uv) strength set, then with a zero strength set, then under two
  different luma directions
- **THEN** the nonzero-uv set derings the chroma ripple (changing it, bounded
  within the original band) while leaving luma byte-identical (the chroma strengths
  are chroma-only)
- **AND** the zero-uv set leaves the chroma byte-identical (a chroma no-op)
- **AND** with `uv_pri != 0` the chroma output depends on the luma direction (the
  `Cdef_Uv_Dir` selection maps `yDir` to a primary chroma direction), whereas with
  `uv_pri == 0` the direction is forced to 0 and the luma direction is ignored

#### Scenario: chroma strengths outside the admitted subset still reject
- **WHEN** a CDEF-active frame with nonzero chroma strengths also has
  `CdefStrengths > 1`, is 10-bit, uses non-4:2:0 chroma subsampling, or has
  multiple tiles
- **THEN** the decoder rejects it before any caller-visible output with a
  structured `decode/unsupported-feature` diagnostic, because no oracle fixture
  pins those shapes
