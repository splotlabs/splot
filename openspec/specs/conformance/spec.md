# conformance Specification

## Purpose

Proof that the implementation matches the spec: no-panic fuzzing, inspector
snapshots, public vectors, and differential testing against AVM (the oracle).

Tracked by Feature IDs: `CONF-FUZZ-NO-PANIC`, `CONF-INSPECT-SNAPSHOTS`,
`CONF-PUBLIC-VECTORS`, `CONF-AVM-DIFF-HARNESS`.
## Requirements
### Requirement: parsers never panic

Arbitrary input SHALL never panic the parsers. This is covered on stable by the
`parsers_never_panic` proptest and, on nightly, by the `parse_obu` cargo-fuzz target.

#### Scenario: arbitrary bytes

- **WHEN** any byte slice is passed to the LEB128, OBU-header, and Annex B parsers
- **THEN** each returns `Ok`/`Err`, never panicking

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

### Requirement: every untrusted-input surface has fuzz coverage

Every public entry point that consumes arbitrary bytes SHALL be reachable
from at least one cargo-fuzz target: the descriptor and OBU-envelope readers,
the IVF container parser, the container auto-detect, and the `splot-validate`
validator API (which transitively dispatches every OBU payload parser). The
CI fuzz-smoke job SHALL enumerate and run every target rather than a
hardcoded subset.

#### Scenario: a payload parser panics on hostile input

- **WHEN** any OBU payload parser reachable from `Validator::validate_bytes`
  panics, hangs, or exceeds the RSS limit on a fuzzer-generated input
- **THEN** the `validate_bytes` fuzz target crashes and the blocking CI
  fuzz-smoke job fails

#### Scenario: a new fuzz target is added

- **WHEN** a new target is added under `fuzz/fuzz_targets/`
- **THEN** the CI fuzz-smoke job and `cargo xtask fuzz` pick it up without a
  workflow edit (targets are enumerated, not hardcoded)

### Requirement: validator no-panic property tests on stable

`splot-validate` SHALL have property tests asserting that validating arbitrary
bytes under arbitrary validator options returns a report and never panics, so
the no-panic invariant is enforced on the stable toolchain in `cargo test`
where nightly-only fuzzing is unavailable.

#### Scenario: arbitrary input on stable

- **WHEN** `cargo test -p splot-validate` runs the property tests on the
  pinned stable toolchain
- **THEN** `Validator::validate_bytes_with_options` returns a
  `ValidationReport` for every generated input without panicking

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

