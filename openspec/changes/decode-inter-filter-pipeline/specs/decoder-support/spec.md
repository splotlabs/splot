## MODIFIED Requirements

### Requirement: First-inter-frame frontier in-loop filter subset
The decoder SHALL apply the § 7.2 final filter chain — § 7.17 deblocking,
§ 7.18 CDEF, § 7.19 CCSO, § 7.20 loop restoration, in that order with the
spec's CurrFrame/CdefFrame snapshot semantics — to reconstructed inter
frames before output and reference storage, deriving deblock geometry from
the decoded transforms (`Max_Tx_Size_Rect` tiling for skip blocks) and the
CDEF/CCSO unit grids and LR source blocks from the parsed tile syntax. For
a WARP_NEWMV block it SHALL read `use_extend_warp` and `use_local_warp`
per § 5.20.7.14 gated on § 7.11.4 `WarpSampleFound[ 0 ]` and the
frame-enabled motion modes, deferring EXTENDWARP/LOCALWARP prediction with
a structured diagnostic. It SHALL read `cctx_type` for inter chroma
transforms per § 5.20.7.27, deferring a nonzero value, and SHALL defer
frames using § 5.18.7.12 `reuse_ccso` or `sb_reuse_ccso`. The committed
deblock-active, CDEF-active, and CCSO-active inter fixtures SHALL decode
byte-identical to `avmdec --i420 --rawvideo`.

#### Scenario: Deblock-active inter frame decodes bit-exact
- **GIVEN** `syn-2frame-deblock-inter-32x32-10bit-q100.ivf` (inter frame
  with all four deblocking passes active)
- **WHEN** the stream is decoded
- **THEN** both frames match the pinned avmdec-verified hashes

#### Scenario: CCSO reference reuse defers
- **GIVEN** an inter frame whose CCSO plane signals `reuse_ccso` or
  `sb_reuse_ccso`
- **WHEN** the frame header is validated
- **THEN** decode rejects with `inter_ccso_reuse_unimplemented` before
  any output
