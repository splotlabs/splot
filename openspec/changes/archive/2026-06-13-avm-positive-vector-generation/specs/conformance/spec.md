# conformance delta: avm-positive-vector-generation

Advances `CONF-AVM-VALID-STREAMS` (diverse positive-vector coverage).

## ADDED Requirements

### Requirement: diverse positive-vector coverage

The committed conformance corpus SHALL include AVM-generated valid streams
(from project-owned synthetic input) spanning diverse codec feature
combinations — at least multiple resolutions, an 8-bit and a 10-bit stream,
intra-only and inter, and an operating-point-set stream — each validated
against the manifest by the committed runner with no AVM dependency. Streams
AVM produces for external-HLS provision (an absent global LCR, or a QM level
with no QM OBU) MAY be committed with their standalone-validation diagnostic as
the expected outcome.

#### Scenario: a diverse clean stream validates clean

- **WHEN** the runner validates a committed self-contained AVM stream (e.g. the
  10-bit intra or the operating-point-set stream)
- **THEN** the validator reports no errors

#### Scenario: an external-HLS-dependent stream emits its availability diagnostic

- **WHEN** the runner validates a committed AVM stream that references a
  resource AVM expects to be provided externally (a global LCR, or a QM level),
  with external HLS disabled
- **THEN** the validator emits exactly that resource's availability diagnostic
