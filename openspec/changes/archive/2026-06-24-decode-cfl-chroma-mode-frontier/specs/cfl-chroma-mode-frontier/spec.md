## ADDED Requirements

### Requirement: local decoder mission CfL Chroma Mode Frontier
The decoder SHALL track `DECODE-CFL-CHROMA-MODE-FRONTIER` as a partial
runtime prerequisite for the local decoder mission Wiener NS LR path. When the supported
non-lossless 4:2:0 intra path reaches AV2 §5.20.5.6 `read_intra_uv_mode()` and
the block selects `UV_CFL_PRED`, the runtime SHALL consume the active `is_cfl`
symbol, represent the typed chroma mode as `UV_CFL_PRED`, and then consume AV2
§5.20.7.32 `read_cfl_alphas()` using the generated AV2 §9.3 CDF rows exposed
through the tile CDF subset. The runtime SHALL remain fail-closed before CfL
prediction or decoded chroma sample output.

#### Scenario: Active CfL syntax advances the transform-record frontier
- **WHEN** the local decoder mission stream reaches active Wiener NS LR
  selectable transform-record derivation
- **AND** a supported intra block selects `UV_CFL_PRED`
- **THEN** the runtime consumes the `is_cfl` and `read_cfl_alphas()` syntax in
  spec order
- **AND** it no longer emits
  `unsupported_wienerns_lr_live_transform_record_cfl_mode` for that block
- **AND** it stops at the next structured unsupported frontier before output

#### Scenario: CfL alpha CDF rows are lifecycle-managed
- **WHEN** tile CDF defaults are created, selected, averaged, or frame-end scaled
- **THEN** the CfL index, sign, alpha, MHCCP, and MH direction CDF rows use the
  generated AV2 §9.3 defaults and the same checked selector/lifecycle paths as
  other block-symbol rows

#### Scenario: No CfL reconstruction claim
- **WHEN** active CfL mode and alpha syntax have been consumed
- **THEN** the decoder SHALL NOT claim `CflRef` capture, CfL prediction,
  chroma reconstruction, 10-bit output, loop-restoration filtering, reference
  refresh, AVM/dav2d byte equality, or successful local decoder mission decode
