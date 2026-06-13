# validator delta: annex-a-iop-layer-budget

Advances `AV2-A-PROFILES` by enforcing the Annex A Table A.3 interoperability-point layer
budget (one of the four remaining profile residuals).

## ADDED Requirements

### Requirement: Annex A interoperability-point layer budget

The validator SHALL enforce the Annex A Table A.3 layer budget for a coded (multistream)
video sequence's table-determined interoperability point
(docs/spec/av2/1.0.0/annex-a-profiles-levels-and-tiers.md, Table A.3, mirror lines 125-170),
emitting `annex-a/layer-budget-exceeds-iop` when the Number of Extended Layers exceeds the
maximum (4), the Number of Embedded Layers exceeds the per-IOP maximum (1 for IOP0, 2 for
IOP1, 3 for IOP2), or the Extended-and-Embedded combination (more than one of each) occurs at
IOP0 or IOP1 (where Table A.3 forbids it; IOP2 permits it). The extended- and embedded-layer
counts are conservative lower bounds (under-counted when activations are missing, never
over-counted), so a count exceeding its limit is a proven violation. The check runs only when
the interoperability point is table-determined (a reserved / Configurable / disagreeing
profile is skipped) and is suppressed under any Provided external-HLS mode with the rest of
the IOP window.

#### Scenario: embedded-layer budget exceeded

- **WHEN** an IOP0 coded video sequence declares more than one embedded layer
- **THEN** an error diagnostic `annex-a/layer-budget-exceeds-iop` (§ A.2) is produced

#### Scenario: forbidden extended-and-embedded combination

- **WHEN** an IOP1 coded video sequence has both more than one extended layer and more than
  one embedded layer
- **THEN** an error diagnostic `annex-a/layer-budget-exceeds-iop` (§ A.2) is produced

#### Scenario: within budget stays silent

- **WHEN** a coded video sequence's layer counts are within its interoperability point's
  Table A.3 budget (including an IOP2 Extended-and-Embedded combination, which IOP2 permits)
- **THEN** no `annex-a/layer-budget-exceeds-iop` diagnostic is produced

#### Scenario: external-HLS suppression

- **WHEN** validation runs under any Provided external-HLS mode
- **THEN** the layer-budget check does not fire

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
