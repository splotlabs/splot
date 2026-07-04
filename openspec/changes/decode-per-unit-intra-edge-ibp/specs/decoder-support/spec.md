## MODIFIED Requirements

### Requirement: Per-unit intra edge filters and IBP blend
The decoder SHALL, for intra-coded transform units reconstructed
through the per-unit residual pipeline, resolve the § 7.13.2.7 edge and
corner filters from the recorded neighbour smoothness (inter neighbours
contributing `false` per the § 5.20 `YModes` semantics), SHALL apply
the § 7.13.2.9 IBP second-directional-predictor blend on one-sided
even-angle-delta luma units at `MrlIndex == 0` with `applyIbp` derived
from the transform-unit size, SHALL re-derive each mrl-0 directional
unit's `pAngle` through the § 5.20.7.29 WAIP wide-angle remap with the
unit's own dimensions and re-select the zone arm accordingly, and SHALL
predict fractional-vector IntraBC blocks through the § 7.13.3.18
separable convolution with the BILINEAR filter row over the current
frame instead of deferring or copying unfiltered samples.

#### Scenario: Mission-stream pre-filter luma matches AVM
- **GIVEN** the local decoder mission stream's first three coded frames
- **WHEN** each frame's pre-filter workspace is compared to the AVM
  oracle's pre-filter dump
- **THEN** every luma sample is byte-identical, including the
  intra-in-inter directional units and the fractional-vector IntraBC
  block

#### Scenario: Admitted corpus stays byte-exact
- **GIVEN** the AVM differential corpus of admitted inter streams
- **WHEN** each stream decodes end to end
- **THEN** the raw output is byte-identical to avmdec
