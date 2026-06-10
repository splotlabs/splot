# validator spec delta

## ADDED Requirements

### Requirement: Distinct embedded-layer count per coded video sequence

`splot-validate` SHALL count the distinct `obu_mlayer_id` values observed in each
extended layer's coded video sequence (AV2 v1.0.0 § 6.4.1: counting applies to all
OBUs, even non-layer-specific ones) and SHALL emit
`sequence-state/distinct-mlayer-count-exceeds-seq-max` (severity `error`) when the
count exceeds the active sequence header's `SeqMaxMlayerCnt`. Counting SHALL reset at
each § 7.3.6 CVS start (CLK) for that extended layer; because the new coded video
sequence starts AT the temporal unit containing the CLK, the same-temporal-unit ids
observed before the CLK (canonically the § 7.3.8.1 resent-at-RAP sequence header,
forced to `obu_mlayer_id == 0`) SHALL be re-attributed to the new coded video sequence
and counted toward its `SeqMaxMlayerCnt`. Ids from temporal units before the boundary
temporal unit SHALL NOT count into the new coded video sequence, and OBUs whose
attribution to the CVS is ambiguous under the documented reading (global
`obu_xlayer_id`) SHALL NOT be counted (sound under-approximation).

#### Scenario: distinct mlayer ids exceed SeqMaxMlayerCnt

- **GIVEN** an active sequence header with `seq_max_mlayer_cnt_minus_1 == 0`
  (`SeqMaxMlayerCnt == 1`)
- **WHEN** OBUs of the same coded video sequence carry two distinct `obu_mlayer_id`
  values
- **THEN** validation SHALL emit `sequence-state/distinct-mlayer-count-exceeds-seq-max`
  with spec section § 6.4.1.

#### Scenario: count resets at a CVS boundary

- **GIVEN** a coded video sequence using `SeqMaxMlayerCnt` distinct `obu_mlayer_id`
  values
- **WHEN** a CLK starts a new CVS for the extended layer and the new CVS uses a
  disjoint but equally sized set of `obu_mlayer_id` values
- **THEN** validation SHALL NOT emit
  `sequence-state/distinct-mlayer-count-exceeds-seq-max`.

#### Scenario: pre-CLK header is re-attributed to the new CVS

- **GIVEN** an active sequence header with `SeqMaxMlayerCnt == 1`
- **WHEN** a single temporal unit carries a resent sequence header at
  `obu_mlayer_id == 0` followed by a CLK at `obu_mlayer_id == 1` (the CLK begins a new
  coded video sequence at that temporal unit)
- **THEN** validation SHALL count `{0, 1}` toward the new CVS and emit
  `sequence-state/distinct-mlayer-count-exceeds-seq-max` exactly once with spec section
  § 6.4.1.

### Requirement: SWITCH and RAS frame dependency-map self-containment

`splot-validate` SHALL emit
`frame-header/switch-or-ras-mlayer-dependency-not-self-contained` (severity `error`)
when a frame-bearing OBU with `obu_type` equal to `OBU_SWITCH` or `OBU_RAS_FRAME` has,
for any embedded layer ID `m` not equal to its `obu_mlayer_id`,
`MLayerDependencyMap[obu_mlayer_id][m] != 0` in the active sequence header
(AV2 v1.0.0 § 6.4.1).

#### Scenario: switch frame depends on another embedded layer

- **GIVEN** an active sequence header whose `MLayerDependencyMap` marks embedded layer
  1 as depending on embedded layer 0
- **WHEN** an `OBU_SWITCH` with `obu_mlayer_id == 1` is validated
- **THEN** validation SHALL emit
  `frame-header/switch-or-ras-mlayer-dependency-not-self-contained` with spec section
  § 6.4.1.

#### Scenario: self-contained RAS frame passes

- **GIVEN** an active sequence header whose `MLayerDependencyMap` row for embedded
  layer 1 references only embedded layer 1
- **WHEN** an `OBU_RAS_FRAME` with `obu_mlayer_id == 1` is validated
- **THEN** validation SHALL NOT emit
  `frame-header/switch-or-ras-mlayer-dependency-not-self-contained`.

### Requirement: Single active sequence header per extended layer per CVS

`splot-validate` SHALL emit `hls/multiple-active-sequence-headers` (severity `error`)
when, within one extended layer's coded video sequence, a frame-confirmed sequence
activation is followed by a non-CLK activation of a different `seq_header_id` with no
intervening CVS start (AV2 v1.0.0 § 7.3.6: within each extended layer, only one
sequence header remains active for the duration of a coded video sequence). The check
SHALL NOT fire when the prior activation was only an OBU-order fallback guess, and
SHALL be suppressed only when caller-provided external HLS declares at least one
sequence header (an external channel that declares no sequence header cannot supply an
out-of-band active header, so it SHALL NOT suppress).

#### Scenario: second activation without a CLK

- **GIVEN** a frame-confirmed activation of `seq_header_id == 0` for an extended layer
- **WHEN** a later non-CLK frame header in the same CVS activates `seq_header_id == 1`
  for that extended layer
- **THEN** validation SHALL emit `hls/multiple-active-sequence-headers` with spec
  section § 7.3.6.

#### Scenario: re-activation across a CLK is conforming

- **GIVEN** a frame-confirmed activation of `seq_header_id == 0` for an extended layer
- **WHEN** a CLK starts a new CVS for that extended layer and its frame header
  activates `seq_header_id == 1`
- **THEN** validation SHALL NOT emit `hls/multiple-active-sequence-headers`.

#### Scenario: unreferenced extra sequence header is conforming

- **GIVEN** an active sequence header for an extended layer
- **WHEN** a sequence-header OBU with a different `seq_header_id` appears in the
  bitstream without being referenced by any frame header
- **THEN** validation SHALL NOT emit `hls/multiple-active-sequence-headers`
  (§ 7.3.6 permits unactivated additional sequence headers).

### Requirement: Monotonic output order agreement across a CMVS

`splot-validate` SHALL track § 7.3.2 coded-multistream-video-sequence boundaries with a
three-state tracker (`Outside` / `Inside` / `Unknown`) and SHALL emit
`sequence-state/monotonic-output-order-mismatch` (severity `error`) when, definitively
inside a CMVS, extended layers are associated with active sequence headers that
disagree on `monotonic_output_order_flag` (AV2 v1.0.0 § 6.4.1). The check SHALL NOT
fire in the `Outside` or `Unknown` tracker states. When a disagreement is observed at a
sequence-header OBU before any CLK in the temporal unit, the `Inside` membership is only
provisional (a later MSDO-less CLK could end the CMVS, § 7.3.2 end condition 2), so the
emission SHALL be deferred to temporal-unit completion and dropped if the temporal unit
turns out to end the CMVS or become `Unknown`. The check SHALL be suppressed only when
caller-provided external HLS declares at least one sequence header (consistent with the
distinct-mlayer and active-sequence-limit gates); an external channel declaring no
sequence header SHALL NOT suppress it.

#### Scenario: flag disagreement inside a CMVS

- **GIVEN** a CMVS begun by a temporal unit containing a CLK with an accompanying MSDO
- **AND** two extended layers whose activated sequence headers disagree on
  `monotonic_output_order_flag`
- **WHEN** the second of the two headers is activated
- **THEN** validation SHALL emit `sequence-state/monotonic-output-order-mismatch` with
  spec section § 6.4.1.

#### Scenario: disagreement outside any CMVS is not flagged

- **GIVEN** two independent extended layers with no MSDO and no global layer
  configuration record in the bitstream
- **WHEN** their activated sequence headers disagree on `monotonic_output_order_flag`
- **THEN** validation SHALL NOT emit `sequence-state/monotonic-output-order-mismatch`.

#### Scenario: provisional-Inside redefinition before a CMVS-ending CLK is not flagged

- **GIVEN** a CMVS that is committed `Inside`
- **WHEN** a temporal unit begins with a same-CVS sequence-header redefinition that
  disagrees on `monotonic_output_order_flag`, followed by an MSDO-less CLK that ends the
  CMVS for that temporal unit (§ 7.3.2 end condition 2)
- **THEN** validation SHALL NOT emit `sequence-state/monotonic-output-order-mismatch`
  (the provisional verdict is deferred and dropped once the CLK is observed).
