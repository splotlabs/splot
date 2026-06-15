# encoder-tools delta: frame-header-writer-segmentation

## ADDED Requirements

### Requirement: frame-header segmentation writer

`splot-core` SHALL provide a writer that is the exact inverse of the § 5.18.7.1
`segmentation_params()` parser on the intra frame-header path. For every model the writer
accepts, reparsing the written bits with the same sequence-derived (`CoreSeqSegView`) and
resolved-multi-frame-header (`MfhSegView`) inputs SHALL yield the original
(`parse(write(x)) == x`). The writer SHALL be additive (no model or parser-error change;
only a visibility-only re-export widen) and SHALL never panic: a model the parser could not
have produced SHALL be rejected with a typed writer error before any bit is written.

The writer SHALL emit fields in the parser's § 5.18.7.1 read order — `segmentation_enabled`
`f(1)` always; when enabled, `reuse_seg_info` `f(1)` only when `allowChange`; on the fresh
path the `seg_info(MaxSegments)` body via the shared § 5.4.9 segment-info writer; and no
bits on the reuse path. Every value the parser derives rather than reads —
`reuse_seg_info` when `allowChange == 0`, the reuse `features` copy, the intra-inferred
`segmentation_update_map` / `segmentation_temporal_update`, and `SegIdPreSkip` /
`LastActiveSegId` — SHALL be re-derived and validated, never coded.

#### Scenario: each segmentation branch round-trips

- **WHEN** a parsed `segmentation_params()` structure is written with the same `seg` / `mfh`
  gating inputs and reparsed
- **THEN** the reparsed structure SHALL equal the original, across every branch (disabled;
  enabled with `reuse_seg_info` inferred or coded; the fresh `seg_info()` body; and the MFH
  arm, the sequence arm, and the zero fallback for the reuse source).

#### Scenario: a non-reproducible segmentation model is rejected before any bit

- **WHEN** a model carries an inferred field that disagrees with its derivation (a
  `reuse_seg_info` not equal to `haveSegParams` when `allowChange == 0`, a reuse `features`
  table not equal to the reuse source, a `segmentation_update_map` / `segmentation_temporal_update`
  not matching the intra-path inferred constants, or a `SegIdPreSkip` / `LastActiveSegId`
  not matching the feature-table re-derivation), or a disabled model carrying any non-default
  field
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.
