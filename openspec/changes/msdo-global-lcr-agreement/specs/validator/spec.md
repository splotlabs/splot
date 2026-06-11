# validator delta: msdo-global-lcr-agreement

Advances `AV2-5.8.1-LCR-GLOBAL-INFO`, `AV2-7.3.2-CMVS-BOUNDARIES`, and
`AV2-A-PROFILES` (the Table A.4 re-land).

## ADDED Requirements

### Requirement: MSDO and activated global LCR agreement

The validator SHALL enforce the § 6.8.2 agreement constraints when an
OBU_MSDO and an activated global layer configuration record are present in
the same coded multistream video sequence, evaluated when CMVS membership is
final: stream-count equality, sub-xlayer containment, aggregate-info
consistency (Annex A.3/A.1 mappings, level, tier), per-substream PTL
equality, and DOH-flag equality. An observed but never-activated global LCR
SHALL trigger none of these.

#### Scenario: stream count disagrees

- **WHEN** a CMVS contains an MSDO with `num_streams_minus_2 + 2 = 2` and an
  activated global LCR with `LcrMaxNumXLayerCount = 3`
- **THEN** `lcr/msdo-stream-count-mismatch` (error, § 6.8.2) is emitted

#### Scenario: unactivated global LCR is inert

- **WHEN** a global LCR is observed but no frame-confirmed activation
  resolves to it
- **THEN** no § 6.8.2 agreement diagnostic is emitted

### Requirement: LCR DOH-constraint flag requirement

The validator SHALL emit `lcr/doh-constraint-required` (error, § 6.8.2) when,
with CMVS membership final, any frame-confirmed activated sequence header has
`monotonic_output_order_flag = 0` while the activated global LCR has
`lcr_doh_constraint_flag = 0`.

#### Scenario: non-monotonic layer without the LCR DOH flag

- **WHEN** a CMVS-inside activated header signals
  `monotonic_output_order_flag = 0` and the activated global LCR's
  `lcr_doh_constraint_flag` is 0
- **THEN** `lcr/doh-constraint-required` is emitted

### Requirement: CMVS boundary-set identity

The validator SHALL emit `cmvs/boundary-set-mismatch` (error, § 7.3.2) when
the MSDO-derived coded-multistream-video-sequence boundary set decidably
disagrees with the MSDO-plus-LCR-derived set, and SHALL stay silent in every
Unknown tracker state.

#### Scenario: undecidable stays silent

- **WHEN** the CMVS tracker cannot decide both boundary sets
- **THEN** no boundary diagnostic is emitted

### Requirement: Annex A interoperability-point OBU presence

The validator SHALL enforce the Table A.4 MSDO/LCR presence requirements at
coded-video-sequence scope with: the interoperability point taken from the
MSDO's `multistream_profile_idc` when an MSDO is present, else from
frame-confirmed activated headers; only activated global LCRs satisfying the
global-LCR arms; per-temporal-unit observation attribution that assigns a
CLK-bearing temporal unit's HLS OBUs to the new coded video sequence
(§ 7.3.6); windows spanning the whole coded video sequence; and suppression
when external HLS is provided.

#### Scenario: multi-xlayer stream without MSDO

- **WHEN** a profile-0 CVS contains two distinct non-global `obu_xlayer_id`
  values across its temporal units and no OBU_MSDO
- **THEN** `annex-a/msdo-required-for-iop` (error) is emitted at CVS end

#### Scenario: unactivated global LCR does not satisfy the arm

- **WHEN** an IOP2 CVS requires a global LCR and contains one that is never
  activated
- **THEN** the presence requirement still fails

#### Scenario: pre-CLK MSDO belongs to the new sequence

- **WHEN** a temporal unit carries an OBU_MSDO before the CLK that starts a
  new coded video sequence
- **THEN** the MSDO counts toward the new sequence's window, not the prior
  one

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
