# validator spec delta

## ADDED Requirements

### Requirement: Intra-CVS operating-point buffer-delay sum constancy

`splot-validate` SHALL track the last explicitly signaled
`ops_decoder_buffer_delay + ops_encoder_buffer_delay` sum per
`(obu_xlayer_id, ops_id, operating-point index)` and SHALL emit
`decoder-model/buffer-delay-sum-changed` (severity `error`, AV2 v1.0.0 § 6.10.5
with § 6.4.13) when the same triple is redefined within one coded video sequence,
with no intervening OPS reset, both signalings explicitly carrying decoder-model
info, and a differing sum. Absent decoder-model info (including Annex E
resource-availability defaults) SHALL NOT participate in any comparison, and a
defining OPS that omits decoder-model info for a previously tracked operating
point SHALL clear that triple's stored baseline (Annex E.1 non-persistence).

#### Scenario: intra-CVS OPS redefinition changes the sum

- **GIVEN** an operating point set defining an operating point with explicit
  `ops_decoder_buffer_delay + ops_encoder_buffer_delay == S`
- **WHEN** a later OPS in the same coded video sequence redefines the same
  `(obu_xlayer_id, ops_id, operating-point index)` without an OPS reset, with
  explicit decoder-model info whose sum differs from `S`
- **THEN** validation SHALL emit `decoder-model/buffer-delay-sum-changed` with
  severity `error`.

#### Scenario: redefinition across a CVS boundary is not an error

- **GIVEN** an operating point with an explicit buffer-delay sum
- **WHEN** the same triple is redefined with a different sum after a CLK starts a
  new coded video sequence for that extended layer
- **THEN** validation SHALL NOT emit `decoder-model/buffer-delay-sum-changed`.

#### Scenario: redefinition without explicit decoder-model info is ignored

- **GIVEN** an operating point with an explicit buffer-delay sum
- **WHEN** the same triple is redefined in the same CVS without
  `ops_decoder_model_info_for_this_op_present_flag` set
- **THEN** validation SHALL NOT emit `decoder-model/buffer-delay-sum-changed` and
  SHALL NOT compare against any default values.

### Requirement: Cross-boundary buffer-delay sum advisory

`splot-validate` SHALL emit `decoder-model/buffer-delay-sum-changed-across-cvs`
(severity `warning`, AV2 v1.0.0 § 6.4.13 / § 6.10.5) when explicitly signaled
buffer-delay sums change across a coded-video-sequence or OPS-reset boundary:
either the activated sequence header's `seq_decoder_model_info()` sum changing
across a CLK boundary within the same extended layer (frame-confirmed activations
only), or an operating point's sum changing across a CVS or OPS-reset boundary for
the same triple. The diagnostic message SHALL state that the constraint scope is
ambiguous in the specification and the finding is advisory under the broad
reading. A frame-confirmed activated header that omits `seq_decoder_model_info()`
SHALL clear that extended layer's stored baseline (Annex E.1 non-persistence).

#### Scenario: activated sequence headers disagree across a CLK

- **GIVEN** a frame-confirmed activated sequence header with explicit
  `seq_decoder_model_info()` sum `S` for an extended layer
- **WHEN** a CLK starts a new CVS for that extended layer and its frame-confirmed
  activated header carries explicit decoder-model info with a sum differing from
  `S`
- **THEN** validation SHALL emit
  `decoder-model/buffer-delay-sum-changed-across-cvs` with severity `warning`.

#### Scenario: headers without decoder-model info never fire the advisory

- **GIVEN** consecutive coded video sequences whose activated sequence headers
  omit `seq_decoder_model_info()`
- **WHEN** the validator runs
- **THEN** validation SHALL NOT emit
  `decoder-model/buffer-delay-sum-changed-across-cvs`.

#### Scenario: external HLS suppresses both decoder-model diagnostics

- **GIVEN** validation options with caller-provided external HLS
- **WHEN** any buffer-delay sum change is observed
- **THEN** validation SHALL NOT emit `decoder-model/buffer-delay-sum-changed` or
  `decoder-model/buffer-delay-sum-changed-across-cvs`.
