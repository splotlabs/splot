# conformance delta: conformance-corpus-foundation

Advances `CONF-AVM-VALID-STREAMS` (the committed valid-vector corpus + runner)
and reframes `CONF-AVM-DIFF-HARNESS` (AVM as a local oracle, never a committed
dependency).

## ADDED Requirements

### Requirement: committed conformance corpus

The committed conformance corpus under `tests/conformance/` SHALL be
self-contained and validate without AVM: a manifest maps each committed vector
to its expected validation outcome (clean, or a set of expected diagnostic
`rule_id`s), and a CI-reachable runner SHALL validate every manifest vector
with `splot-validate` and assert its expected outcome. The committed runner,
build, and CI SHALL NOT invoke or depend on AVM; AVM is only the local
generator of the committed vectors.

#### Scenario: committed valid vector validates clean

- **WHEN** the runner validates a committed vector whose manifest entry is
  `clean`
- **THEN** the validator reports no errors and the runner passes

#### Scenario: runner needs no AVM

- **WHEN** CI runs the conformance runner
- **THEN** it validates the committed vectors without invoking AVM or the
  network

## MODIFIED Requirements

### Requirement: AVM as oracle

Differential testing SHALL use AVM as a LOCAL oracle/generator only: AVM
locally produces AV2 streams that `splot validate` must validate clean (or flag
a real defect), and those small generated streams MAY be committed as plain
project fixtures. AVM SHALL NOT be vendored and SHALL NOT be a build or CI
dependency (no committed code path invokes AVM). Proof of the committed
vectors is recorded in the relevant matrix row's `[feature.proof]`.

#### Scenario: AVM-produced stream

- **WHEN** AVM locally encodes a stream and `splot validate` runs on it
- **THEN** the stream validates clean or a real defect is reported

### Requirement: vector licensing

Only redistributable/public vectors SHALL be committed: AVM-generated AV2
bitstreams (AVM is BSD-3-Clause-Clear) MAY be committed as project fixtures,
and samples whose license is unclear SHALL NOT be committed.

#### Scenario: unclear-license sample

- **WHEN** a vector's license is unclear
- **THEN** it is NOT committed to the repository
