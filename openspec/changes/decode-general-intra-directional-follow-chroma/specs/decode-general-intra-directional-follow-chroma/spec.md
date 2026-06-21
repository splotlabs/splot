## ADDED Requirements

### Requirement: General intra single-block directional-follow (D135) chroma decode
The decoder SHALL reconstruct a full-superblock (`n4w == 16`) top-left
(no-neighbour) general intra block whose chroma resolves to the § 7.13.2.8
`D135_PRED` directional-follow mode. When AV2 § 5.20.5.3 `read_intra_uv_mode`
decodes `uv_mode == 0` over a directional luma, `get_intra_uv_mode_set(0)` returns
`YMode` and the spec sets `AngleDeltaUV = AngleDeltaY`; for the supported luma
`D135_PRED` (`AngleDeltaY == 0`) the resolved chroma is `UVMode == D135_PRED`,
`AngleDeltaUV == 0`. The decoder SHALL admit this chroma mode (no longer rejecting
it as an unsupported non-DC chroma mode) ONLY on the directional-follow branch
(`uv_mode == 0` and the luma is directional); a `D135_PRED` value resolved from the
`Default_Mode_List_Uv` scan paired with a non-directional luma SHALL remain
deferred. For the no-neighbour top-left block the decoder SHALL build the
§ 7.13.2.1 chroma edges as the flat fallbacks (8-bit: `AboveRow[k] = 127`,
`LeftCol[k] = 129`, shared corner `128`), run the § 7.13.2.8 middle-angle
(pAngle 135) prediction — where `dx = dy = Dr_Intra_Derivative[45] = 64` give
`shift == 0` so the chroma IDIF reduces to a sample copy, bit-identical to the
`enableIdif == 0` bilinear predictor — and add the § 5.20.7.27 chroma residual (or
write the bare prediction for an `all_zero` block) for both the U and V planes.
The decoder SHALL gate this directional-follow chroma to the top-left
(no-neighbour) 64x64 superblock (`n4w == 16`) and SHALL reject — with a structured
`decode/unsupported-feature` diagnostic — a neighbour-having directional chroma
block (which needs the real § 7.13.2.8 chroma IDIF 4-tap, since the bilinear
reduction equals IDIF only over a flat edge), other directional chroma angles or a
non-zero `AngleDeltaUV`, CfL (`UV_CFL_PRED`) / CCTX / MHCCP chroma, and the
non-follow `D135_PRED` scan pairing. The reconstruction SHALL be guarded by the
§ 8.2.4 `exit_symbol()` bit-exactness check and SHALL NOT invoke AVM or dav2d.

#### Scenario: A directional-follow D135 chroma block decodes to the oracle
- **WHEN** `splot decode` is given the committed single-block intra key frame
  `syn-dfchroma-intra-64x64-q80.ivf`, whose 64x64 luma block codes as `D135_PRED`
  and whose chroma codes with `uv_mode == 0` (directional-follow D135)
- **THEN** the general intra path reconstructs the luma D135 block and both the U
  and V directional-follow D135 chroma blocks over the § 7.13.2.1 no-neighbour
  fallback edges plus residual, and succeeds
- **AND** the decoded output matches the avmdec (`--rawvideo --i420`) and dav2d
  (`--demuxer ivf`) raw outputs byte-for-byte (md5
  `09fc23f0bced8ab5b9562d6d2478af1c`)
- **AND** the decoded-frame hash is the pinned
  `628b759dcb63356ad3174063652c54d7ebf6f54d1566ab9f1b64b3a74542154f`

#### Scenario: The chroma is a genuine directional reconstruction, not flat DC
- **WHEN** the directional-follow D135 chroma block is reconstructed over the flat
  fallback edges
- **THEN** the U and V planes each reconstruct as a 135-degree anti-diagonal
  pattern with many distinct values (the upper-right triangle, where the column
  index exceeds the row index, sits below the lower-left triangle), proving the
  § 7.13.2.8 directional chroma predictor ran rather than the DC fallback

#### Scenario: A neighbour-having directional chroma block is rejected
- **WHEN** a general intra block resolves to the directional-follow D135 chroma
  mode at a superblock position that is not the top-left (no-neighbour) block
- **THEN** the decoder emits a structured `decode/unsupported-feature` diagnostic
  (the neighbour-having directional chroma needs the real § 7.13.2.8 chroma IDIF
  4-tap interpolation, not yet implemented) rather than decoding over an unverified
  prediction

#### Scenario: CfL / CCTX / MHCCP chroma stays rejected
- **WHEN** a general intra block's chroma resolves to `UV_CFL_PRED` or reads CCTX /
  MHCCP cross-component syntax
- **THEN** the decoder rejects it with a structured `decode/unsupported-feature`
  diagnostic, because those are not a plain § 7.13 intra prediction and remain out
  of scope

#### Scenario: Existing general intra fixtures still decode bit-exact
- **WHEN** `splot decode` is given the committed general intra fixtures, including
  the original `syn-hedge-intra-64x64-q80.ivf` (D135 luma with `uv_mode == 1` DC
  chroma)
- **THEN** each reconstructs to its previously pinned decoded-frame hash, unchanged
  by adding the directional-follow chroma path
