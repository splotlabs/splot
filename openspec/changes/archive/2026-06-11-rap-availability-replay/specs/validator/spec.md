# validator delta: rap-availability-replay

Advances `AV2-7.3.8-HLS-AVAILABILITY` and the availability halves of
`AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` / `AV2-5.7-MULTI-FRAME-HEADER`.

## ADDED Requirements

### Requirement: HLS availability replays from random access points

The validator SHALL verify, for every § 7.4.1 random access point, that each
HLS OBU referenced at or after it was (re)sent in or after the random access
point's temporal unit (§ 7.3.8.1) — a resend inside a temporal unit carrying
LEADING_* frame OBUs SHALL NOT qualify (those temporal units drop under
random access), while temporal units whose leading-ness is undecidable SHALL
qualify (under-approximation, never a false positive). Externally-declarable
kinds follow the documented partial-declaration suppression policy.

#### Scenario: sequence header only before the RAP

- **WHEN** a frame after a CLK references a sequence header last sent in an
  earlier temporal unit and not resent in the CLK's temporal unit
- **THEN** a replay diagnostic citing § 7.3.8.1 is emitted

#### Scenario: resend in the RAP temporal unit passes

- **WHEN** the referenced HLS OBU is resent inside the random access point's
  temporal unit
- **THEN** no replay diagnostic is emitted

#### Scenario: leading-TU resend does not qualify

- **WHEN** the only post-RAP resend sits in a temporal unit carrying an
  OBU_LEADING_* frame
- **THEN** the replay diagnostic is emitted

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
