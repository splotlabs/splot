# bitstream delta: annex-e-decoder-model-schedule

Advances `AV2-E-DECODER-MODEL` (the § E.7.8 decoding-schedule-mode
`DecoderBufferDelay` bound) and the rate-column modeling on
`AV2-A-LEVELS-TIERS` it depends on. The remaining Annex E.5/E.6/E.7
simulation and the Annex A.4 dynamic rate constraints are carved out as
named residuals below — they are not promised by this change because their
inputs (`CodedBits[i]`, `Removal[]`, `TimeToDecode[i]`, per-second output
durations from the resource-availability buffer-pool replay) are not soundly
decidable from parsed syntax today.

## ADDED Requirements

### Requirement: Annex E.7.8 schedule-mode decoder-buffer-delay bound

The validator SHALL flag a `decoder_buffer_delay` that violates Annex E.7.8
only when a frame-confirmed activated sequence header is in the
decoding-schedule mode for its extended layer, which § E.4.2 requires ALL THREE
signaled parameters to establish: `decoder_model_info_present_flag == 1`,
`seq_decoder_model_info_present_flag == 1`, AND
`ci_timing_info_present_flag == 1` in the content interpretation OBU associated
with the extended layer (established at/after the layer's current § 7.3.8.11
random-access-point epoch). The delay must not be
equal to 0 and must not exceed `90000 * (BufferSize / BitRate)`, which equals
exactly 90000 for every level/tier with a defined bitrate (the
`BitrateProfileFactor` and `MaxBitrate` cancel via the Table A.9 / Annex E.3
identities). The validator SHALL make no such judgment when the bitrate is
undefined for the activated `seq_level_idx` / `seq_tier` (a reserved level, the
Maximum-parameters level `seq_level_idx == 31` per the § E.7.1 exemption, or the
High tier below level 4.0), when any of the three § E.4.2 schedule-mode
conditions is unmet (including when `ci_timing_info_present_flag == 1` is not
established for the layer — never signaled, or reset to 0 by a random access
point and not re-established), or when its activation is not frame-confirmed.

This requirement covers only the extended-layer arm of § E.7.8. The OPS per-op
arm, the rest of the § E.5 frame-timing simulation, and the other § E.7
conformance expressions (E.7.1 availability / presentation monotonicity /
signaled-BRT floor, E.7.2 buffer-delay-across-RAP, E.7.3 overflow, E.7.4
underflow, E.7.5 minimum decode time, E.7.6 minimum presentation interval,
E.7.7 decode deadline) are **named residuals**: they require the Annex E.5.5 /
E.6 resource-availability buffer-pool simulation (`CodedBits[i]` DFG byte
accounting, `Removal[]`, `TimeToDecode[i]`, `DecoderRefCount` /
`PlayerRefCount`) whose inter-frame inputs route to Unknown on the current
parse paths, so firing them would risk false positives.

#### Scenario: schedule-mode zero delay is flagged

- **WHEN** an activated, frame-confirmed sequence header signals schedule mode
  with `decoder_buffer_delay == 0` at a level/tier with a defined bitrate
- **THEN** `decoder-model/schedule-decoder-buffer-delay-zero` fires with its
  § E.7.8 citation

#### Scenario: schedule-mode over-bound delay is flagged

- **WHEN** an activated, frame-confirmed sequence header signals schedule mode
  with `decoder_buffer_delay > 90000` at a level/tier with a defined bitrate
- **THEN** `decoder-model/schedule-decoder-buffer-delay-exceeds-bound` fires
  with its § E.7.8 citation

#### Scenario: undecidable or exempt input stays silent

- **WHEN** the level is the Maximum-parameters level (`seq_level_idx == 31`), a
  reserved level, or the High tier below 4.0, OR the header is not in schedule
  mode because `ci_timing_info_present_flag == 1` is not established for the
  layer (no content interpretation OBU, a content interpretation OBU with
  `ci_timing_info_present_flag == 0`, or a pre-random-access-point CI whose
  `ci_timing` was reset to 0 by a CLK/OLK and not re-established), OR another
  § E.4.2 schedule-mode flag is unset, OR its activation is not frame-confirmed
- **THEN** no decoder-model schedule judgment is made

### Requirement: Annex A.4 bitrate definedness modeling

The validator SHALL model whether the Annex A.4 / Table A.9 bitrate variable
`BitRate` is defined for a `(seq_level_idx, seq_tier)` pair — `MainMbps` is
present for every defined level `0..=21`; `HighMbps` only for level `4.0` and
above — so the § E.7.8 schedule-mode bound can honest-stop when the bitrate is
undefined. The literal Mbps cell values are not transcribed because the
§ E.7.8 bound cancels to a constant; the other rate columns
(`MaxDisplayRate` / `MaxDecodeRate` / `MaxHeaderRate` / `MainCR` / `HighCR`) and
the Annex A.4 dynamic rate constraints that would consume them remain named
residuals (they depend on `Removal[]` / `FrameParsingTime` from the unmodeled
resource-availability simulation).

#### Scenario: defined and undefined bitrate cells

- **WHEN** the bitrate definedness is queried for a Main-tier level `0..=21`,
  a High-tier level `>= 4`, or a reserved/Maximum-parameters/out-of-range level
- **THEN** the first two are reported defined and the last undefined, matching
  the Table A.9 Mbps columns
