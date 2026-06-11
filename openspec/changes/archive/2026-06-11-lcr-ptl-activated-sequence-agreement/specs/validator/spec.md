# validator delta: lcr-ptl-activated-sequence-agreement

Advances `AV2-5.8.4-LCR-SEQ-PTL-INFO` and `AV2-5.8.7-LCR-REP-INFO`.

## ADDED Requirements

### Requirement: LCR PTL ceilings constrain activated headers

The validator SHALL enforce the § 6.8.5 ceiling sentences: when
`lcr_seq_profile_tier_level_info(i)` is present in the LCR activated by
extended layer `i`'s frame-confirmed sequence header, the header's
`seq_profile_idc`, `seq_level_idx`, `seq_tier`, and
`seq_max_mlayer_cnt_minus_1 + 1` SHALL each be ≤ the corresponding
LCR-declared maximum, with equality passing and absent PTL info comparing
nothing.

#### Scenario: level exceeds the LCR ceiling

- **WHEN** a frame-confirmed header with `seq_level_idx = 8` activates an
  LCR declaring `lcr_max_level_idx[i] = 4` for its layer
- **THEN** `lcr/ptl-level-exceeds-max` (error, § 6.8.5) is emitted

#### Scenario: equality passes

- **WHEN** the header's value equals the LCR-declared maximum
- **THEN** no ceiling diagnostic is emitted

### Requirement: LCR rep-info equality with activated headers

The validator SHALL enforce the § 6.8.8 equality sentences between an
activated LCR's representation info and each sequence header activated by
the same extended layer (frame dimensions, bit depth, chroma format,
cropping window), emitting `lcr/rep-info-mismatch` (error) naming the
disagreeing field; absent rep-info SHALL compare nothing.

#### Scenario: dimension mismatch

- **WHEN** an activated LCR declares `lcr_max_pic_width = 1920` and the
  activated header has `max_frame_width_minus_1 + 1 = 1280`
- **THEN** `lcr/rep-info-mismatch` (error, § 6.8.8) is emitted naming the
  width field

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
