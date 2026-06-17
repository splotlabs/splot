## ADDED Requirements

### Requirement: Dequantization quantizer-index resolution and per-plane quantizer composition

The repository SHALL provide scheduler-free `splot-recon` primitives for the AV2
§ 7.14.2 quantizer-index resolution and per-plane quantizer composition, tracked
by `RECON-DEQUANT-QUANTIZER-INDEX-RESOLUTION`, extending the § 7.14.2
quantizer-value lookup core. The repository SHALL provide a `quantizer_index`
function implementing § 7.14.2 `get_qindex( ignoreDeltaQ, segmentId )` over
caller-resolved facts: when the alternative-quantizer segment feature is active
it SHALL return `Clip3(0, MaxQ, base + segment_alt_q_data)` where `base` is the
running current quantizer index when `delta_q` applies (not ignore and
`delta_q_present`) and the base quantizer index otherwise, and when the feature
is inactive it SHALL return the current quantizer index when `delta_q` applies
and the base quantizer index otherwise. The repository SHALL provide
`dc_quantizer` and `ac_quantizer` functions implementing § 7.14.2
`get_dc_quant( plane )` and `get_ac_quant( plane )` by selecting the plane's
caller-resolved DC or AC delta (the luma AC delta being 0) and applying the
quantizer-value lookup. The primitives SHALL take caller-resolved inputs, SHALL
be total and panic-free for every input by using widened clamp intermediates,
and SHALL read no frame, segment, or tile state. The primitives SHALL NOT
implement segmentation evaluation, the running quantizer-index update, the
§ 7.14.4 dequantization process, quantizer-matrix weighting, the § 7.14.3
reconstruct process, inverse transforms, residual addition, tile syntax
traversal, runtime decode output, or reference-refresh semantics.

#### Scenario: Quantizer-index resolution succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon dequant --locked` runs
- **THEN** the test suite covers the three `get_qindex` branches, the
  ignore-delta-q override, both clamp bounds at the 8-bit and 10-bit `MaxQ`, the
  per-plane DC and AC selection including the luma AC-0 rule, and an end-to-end
  composition reaching the qindex-0 special case
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Quantizer composition is total and panic-free

- **WHEN** callers pass any base/current quantizer indices, segment data, plane,
  delta offsets, and active bit depth, including out-of-contract extremes
- **THEN** `splot-recon` returns clamped quantizer values computed with widened
  intermediates
- **AND** library code does not panic, overflow, unwrap, or emit `decode/*`
  diagnostics

#### Scenario: Full dequantization remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the quantizer-index resolution and per-plane
  composition as supported
- **AND** broader reconstruction remains partial until the § 7.14.4
  dequantization process, quantizer-matrix weighting, the § 7.14.3 reconstruct
  process, inverse transforms, and residual addition are implemented and proven
