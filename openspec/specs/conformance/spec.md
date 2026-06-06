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

Differential testing SHALL use AVM as the oracle: first `avm encode` →
`splot validate`, and later `splot encode` → `avm decode`. Proof is recorded in the
relevant matrix row's `[feature.proof]`. *Status: planned — tracked by
`openspec/changes/avm-differential-harness`; not yet implemented
(`CONF-AVM-DIFF-HARNESS` is proposed).*

#### Scenario: AVM-produced stream

- **WHEN** AVM encodes a stream and `splot validate` runs on it
- **THEN** the stream validates clean or a real defect is reported

### Requirement: vector licensing

Only redistributable/public vectors may be vendored. Unclear-license samples SHALL
NOT be committed.

#### Scenario: unclear-license sample

- **WHEN** a vector's license is unclear
- **THEN** it is NOT committed to the repository
