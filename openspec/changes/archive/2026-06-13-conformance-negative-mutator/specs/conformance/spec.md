# conformance delta: conformance-negative-mutator

Advances `CONF-AVM-INVALID-STREAMS` (targeted negative vectors with expected
diagnostics).

## ADDED Requirements

### Requirement: targeted negative mutations

The validator SHALL be exercised by a committed, deterministic negative mutator:
for each `(committed valid seed, documented mutation, expected diagnostic)` row,
the mutated stream SHALL produce that registered diagnostic `rule_id` and SHALL
NOT panic. The mutations target stable, decidable diagnostics (IVF container,
OBU header, LEB128 framing); the expected `rule_id`s are existing registered
diagnostics, not new ones, and the mutator runs in CI without AVM or the
network.

#### Scenario: a malformed stream emits its expected diagnostic

- **WHEN** a documented mutation is applied to a committed valid seed and the
  result is validated
- **THEN** the validator emits the row's expected diagnostic `rule_id` and does
  not panic

#### Scenario: a conformant seed without mutation stays clean

- **WHEN** the unmutated seed is validated
- **THEN** the validator reports no errors (the mutation, not the seed, is the
  cause of the diagnostic)
