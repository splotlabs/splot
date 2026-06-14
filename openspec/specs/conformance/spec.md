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

### Requirement: inspect output golden snapshots

The conformance suite SHALL include `insta` golden snapshot tests of the `splot inspect
--json` output over a diverse set of committed fixtures, so any change to the inspector's
per-OBU JSON for a committed fixture is surfaced as a reviewable snapshot diff. The
inspector output is deterministic (per-OBU byte offsets, sizes, and parsed fields, with no
paths, timestamps, or filenames), so the snapshots are stable across runs and machines.

#### Scenario: inspector output is frozen

- **WHEN** `splot inspect --json` is run against a committed fixture
- **THEN** its output matches the committed golden snapshot for that fixture

#### Scenario: an output change is surfaced

- **WHEN** the inspector's JSON output for a committed fixture changes
- **THEN** the snapshot test fails with a diff that must be explicitly reviewed and accepted

### Requirement: CLI help-surface snapshots

The test suite SHALL freeze the `splot validate --help` and `splot inspect --help`
output as committed `insta` golden snapshots, so any change to those subcommands'
argument surface (a new, renamed, removed, or reordered flag, or a changed help
string) is surfaced as a reviewable snapshot diff. The snapshots SHALL be
deterministic — no filesystem paths, timestamps, or version strings — and the
top-level `splot --help` SHALL NOT be snapshotted.

#### Scenario: help surface unchanged

- **WHEN** the committed goldens match the current `validate`/`inspect` `--help`
- **THEN** the snapshot tests pass with no pending snapshots

#### Scenario: a flag is added or renamed

- **WHEN** a `validate` or `inspect` flag is added, renamed, or removed
- **THEN** the corresponding help snapshot diffs, requiring an explicit golden
  update in the same change

### Requirement: inspector text-output snapshots

The test suite SHALL freeze the `splot inspect` human (text) output — both the
default per-OBU dump and the `--headers` header-only dump — as committed `insta`
golden snapshots over representative committed fixtures, complementing the existing
`--json` snapshots. The text output is deterministic for a fixed input.

#### Scenario: text dump is stable

- **WHEN** `splot inspect` (default or `--headers`) runs against a committed fixture
- **THEN** its stdout matches the committed golden exactly

