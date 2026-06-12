# validator delta: reference-state-and-random-access

Completes the header-decidable reference-slot and random-access
conformance tranche.

## ADDED Requirements

### Requirement: random-access reference conformance

The validator SHALL enforce the header-decidable reference-slot and
random-access rules — § 7.3.9.1 long-term reference availability with
the RAP-CELU first-frame-unit rule, the § 7.4.2/.4/.5 long-term, OLK,
and RAS conditions in their header-observable forms, the § 7.3.8.9
quantizer-matrix reference availability with the QmProtected resets, the
§ 6.17.6.2 quantizer-matrix layer-dependency constraints, the § 6.8.9
expected-dimension bounds, and the remaining decidable § 6.17.2
reference clauses — each with its governing citation, firing only on
modeled-state-proven violations and dropping under poisoned or
externally-declared state.

#### Scenario: long-term availability violation

- **WHEN** a frame references a long-term id the modeled state proves
  unavailable
- **THEN** the § 7.3.9.1 diagnostic fires

#### Scenario: poisoned state stays silent

- **WHEN** the slot or long-term state is poisoned or externally
  declared
- **THEN** the dependent judgments drop

#### Scenario: QmProtected reset honored

- **WHEN** a CLK, OLK, RAS, or parsed restricted-prediction SWITCH
  resets the quantizer-matrix protection
- **THEN** the § 7.3.8.9 availability judgment reflects the reset
