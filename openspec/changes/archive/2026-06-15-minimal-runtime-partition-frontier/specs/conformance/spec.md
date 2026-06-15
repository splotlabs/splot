## MODIFIED Requirements

### Requirement: committed conformance corpus

The committed conformance corpus under `tests/conformance/` SHALL be
self-contained and validate without AVM: a manifest maps each committed vector
to its expected validation outcome (clean, or a set of expected diagnostic
`rule_id`s), and a CI-reachable runner SHALL validate every manifest vector
with `splot-validate` and assert its expected outcome. The committed runner,
build, and CI SHALL NOT invoke or depend on AVM; AVM is only the local
generator or source seed described by each vector's manifest provenance.
Repository-retimed vectors SHALL identify the retiming in their manifest
description and SHALL NOT claim local reference-decoder evidence unless that
evidence is refreshed.

#### Scenario: committed valid vector validates clean

- **WHEN** the runner validates a committed vector whose manifest entry is
  `clean`
- **THEN** the validator reports no errors and the runner passes

#### Scenario: runner needs no AVM

- **WHEN** CI runs the conformance runner
- **THEN** it validates the committed vectors without invoking AVM or the
  network

### Requirement: diverse positive-vector coverage

The committed conformance corpus SHALL include valid streams from project-owned
synthetic input, either AVM-generated or explicitly provenance-noted local
retimings, spanning diverse codec feature combinations - at least multiple
resolutions, an 8-bit and a 10-bit stream, intra-only and inter, and an
operating-point-set stream - each validated against the manifest by the
committed runner with no AVM dependency. Streams AVM produces for external-HLS
provision (an absent global LCR, or a QM level with no QM OBU) MAY be committed
with their standalone-validation diagnostic as the expected outcome.

#### Scenario: a diverse clean stream validates clean

- **WHEN** the runner validates a committed self-contained stream (for example
  the 10-bit intra, operating-point-set, or retimed minimal runtime stream)
- **THEN** the validator reports no errors

#### Scenario: an external-HLS-dependent stream emits its availability diagnostic

- **WHEN** the runner validates a committed AVM stream that references a
  resource AVM expects to be provided externally (a global LCR, or a QM level),
  with external HLS disabled
- **THEN** the validator emits exactly that resource's availability diagnostic
