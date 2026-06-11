# validator delta: annex-a-profile-level-skeleton

Advances `AV2-A-PROFILES` and `AV2-A-LEVELS-TIERS` from todo to partial: the
static profile/level/tier subset checkable from parsed sequence/frame/HLS
state. Rate-based and decoder-model constraints stay with the Annex E change.

## ADDED Requirements

### Requirement: Annex A profile constraints

The validator SHALL check the activated sequence header against the AV2
profile definitions (Annex A.2 Table A.1): a reserved `seq_profile_idc`
(5–30), a `chroma_format_idc` outside the profile's allowed set, or a
`bit_depth_idc` outside 0–1 for profiles 0–4 SHALL each produce an error
diagnostic citing Annex A.2. The Configurable profile (31) SHALL NOT be
checked against chroma/bit-depth sets (Table A.1 leaves them unconstrained).

#### Scenario: 4:2:2 under a 4:2:0 profile

- **WHEN** an activated sequence header signals `seq_profile_idc = 0` with
  `chroma_format_idc = CHROMA_FORMAT_422`
- **THEN** `annex-a/profile-chroma-format-mismatch` (error) is emitted

#### Scenario: configurable profile is unconstrained

- **WHEN** an activated sequence header signals `seq_profile_idc = 31` with
  any chroma format
- **THEN** no profile-mismatch diagnostic is emitted

### Requirement: Annex A level and tier value spaces

The validator SHALL flag reserved level indices (Table A.7: 22–30) on
activated `seq_level_idx` and observed `ops_level_idx` values as errors, and
SHALL flag `seq_tier = 1` below level 4.0 as a warning (Table A.9 NOTE — a
non-normative source, hence advisory severity).

#### Scenario: reserved level index

- **WHEN** an activated sequence header signals `seq_level_idx = 25`
- **THEN** `annex-a/level-reserved` (error) is emitted

### Requirement: Annex A static level limits

The validator SHALL enforce the static conformance block of Annex A.4 for a
parsed intra frame header under an activated sequence header whose
`seq_level_idx` maps into Tables A.8/A.9 (not 31, not reserved):
`FrameWidth * FrameHeight <= MaxPicSize`, `FrameWidth <= MaxHSize`,
`FrameHeight <= MaxVSize`, `NumTiles <= MaxTiles`, `TileCols <= MaxTileCols`,
and `FrameWidth, FrameHeight >= 16`, each violation an error diagnostic
citing Annex A.4. Level 31 SHALL disable all of these.

#### Scenario: frame exceeds the level picture size

- **WHEN** a level-2.0 stream carries an intra frame with
  `FrameWidth * FrameHeight > 147456`
- **THEN** `annex-a/frame-size-exceeds-level` (error) is emitted

#### Scenario: maximum-parameters level

- **WHEN** `seq_level_idx = 31`
- **THEN** no level-limit diagnostics are emitted for any frame size

### Requirement: Annex A interoperability-point OBU presence

At coded-video-sequence scope, for profiles 0–4, the validator SHALL enforce
the Table A.4 MSDO/LCR presence requirements using the Table A.3 layer
counting definitions, including both either/or arms of the IOP2 rows, and
SHALL suppress these presence checks when external HLS is provided.

#### Scenario: multi-xlayer IOP0 stream without MSDO

- **WHEN** a profile-0 CVS contains more than one distinct non-global
  `obu_xlayer_id` and no OBU_MSDO
- **THEN** `annex-a/msdo-required-for-iop` (error) is emitted at CVS end

#### Scenario: single-layer stream with MSDO

- **WHEN** a profile-0 CVS with one extended layer contains an OBU_MSDO
- **THEN** `annex-a/msdo-prohibited-for-iop` (error) is emitted

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
