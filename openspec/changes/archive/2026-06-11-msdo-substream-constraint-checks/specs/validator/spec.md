# validator delta: msdo-substream-constraint-checks

Advances `AV2-5.6-MSDO`: the remaining § 6.6 conformance sentences and the
§ 7.3.8.2 non-RAP identity rule, all from already-parsed state.

## ADDED Requirements

### Requirement: MSDO sub-stream PTL floor agreement

The validator SHALL enforce the § 6.6 sub-stream constraint sentences:
`multistream_profile_idc` SHALL be ≥ every `sub_stream_max_profile[i]`, and a
sequence header activated by the i-th sub-stream (frame-confirmed, mapped via
`sub_xlayer_id[i]`) SHALL NOT exceed the declared `sub_stream_max_profile[i]`
/ `sub_stream_max_level[i]` / `sub_stream_max_tier[i]`, in either arrival
order (MSDO before or after the activation).

#### Scenario: substream level exceeds the declared maximum

- **WHEN** an MSDO declares `sub_stream_max_level[0] = 4` for
  `sub_xlayer_id[0] = 1` and a frame-confirmed sequence header with
  `seq_level_idx = 8` activates on extended layer 1
- **THEN** `msdo/substream-level-exceeds-max` (error, § 6.6) is emitted

#### Scenario: equality passes

- **WHEN** the activated header's `seq_level_idx` equals
  `sub_stream_max_level[i]`
- **THEN** no substream-max diagnostic is emitted

### Requirement: MSDO DOH-constraint flag requirement

The validator SHALL emit an error when, definitively inside a coded
multistream video sequence, any frame-confirmed activated sequence header has
`monotonic_output_order_flag = 0` while the recorded MSDO has
`multistream_doh_constraint_flag = 0` (§ 6.6).

#### Scenario: non-monotonic layer without the DOH flag

- **WHEN** a CMVS-inside activated header signals
  `monotonic_output_order_flag = 0` and the MSDO's
  `multistream_doh_constraint_flag` is 0
- **THEN** `msdo/doh-constraint-required` (error, § 6.6) is emitted

### Requirement: non-RAP MSDO identity

The validator SHALL compare each temporal unit's MSDO payload against the
previous MSDO at temporal-unit end and emit an error when the temporal unit
is not a random access point (§ 7.4.1: contains no CLK/OLK/RAS OBU) and the
payloads differ (§ 7.3.8.2). A random-access-point temporal unit SHALL update
the reference payload without a comparison.

#### Scenario: changed MSDO outside a random access point

- **WHEN** a temporal unit without CLK/OLK/RAS carries an OBU_MSDO whose
  payload differs from the previous OBU_MSDO
- **THEN** `msdo/non-rap-not-identical` (error, § 7.3.8.2) is emitted at
  temporal-unit end

#### Scenario: changed MSDO at a random access point

- **WHEN** a temporal unit containing a CLK carries a changed OBU_MSDO
- **THEN** no identity diagnostic is emitted and the reference updates

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
