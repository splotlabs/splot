# Change: validator coverage roadmap

`change-id: validator-coverage-roadmap`  
`status: proposed`  
`owner: validator`  
`kind: roadmap + implementation scaffolding`

## Summary

Expand `splot` from the current Annex B + OBU-header validator into a phased AV2 syntax and semantics validator. This change does not claim to implement the entire AV2 spec in one PR. It adds the roadmap, matrix child rows, diagnostics registry, and coding-agent instructions needed to implement the missing validator features in dependency order.

## Why

The current validator intentionally validates the AV2 length-delimited envelope, LEB128, OBU headers, header-only §6.2.2 rules, and the reserved-OBU all-zero-payload rule. That milestone is valuable, but it still accepts streams whose payload syntax is illegal because most §5 payload parsers and §6/§7 semantics are not implemented.

A complete validator needs a visible plan that is:

- spec-traceable;
- split into implementable rows;
- enforced by `docs/IMPLEMENTATION-MATRIX.toml` and `cargo xtask check-feature-status`;
- careful not to import AV1 assumptions;
- testable with local fixtures, fuzz/property tests, and optional AVM differential tests.

## Scope

This change covers:

1. a validator gap analysis;
2. a phased validator roadmap;
3. matrix row expansion guidance;
4. diagnostic namespace/rule-id guidance;
5. stateful validator architecture for activated sequence headers, HLS availability, and OBU ordering;
6. a coding-agent prompt for implementing the missing validator work;
7. acceptance commands for every implementation phase.

## Non-goals

This change does not implement:

- encoder mode decisions, RDO, rate control, or pixel-domain algorithms;
- full AV2 decoding or reconstruction unless a validator check later requires it;
- hand-transcribed AV2 tables from the PDF;
- copied syntax/tables/code from AV1 implementations;
- mandatory AVM in normal CI.

## Feature IDs touched

Existing rows:

```text
AV2-4.11.6-LEB128
AV2-5.2.1-OBU-TYPE
AV2-5.2.2-OBU-HEADER
AV2-B-ANNEXB-OBU-ENVELOPE
AV2-5.2.3-TRAILING-BITS
AV2-5.2.4-BYTE-ALIGNMENT
AV2-5.3-RESERVED-OBU
AV2-5.4-SEQUENCE-HEADER
AV2-5.8-LAYER-CONFIG-RECORD
AV2-5.10-OPERATING-POINT-SET
AV2-5.18-FRAME-HEADER
AV2-5.19-TILE-GROUP
AV2-7.3-OBU-ORDERING
AV2-9-ADDITIONAL-TABLES
CONF-AVM-DIFF-HARNESS
CONF-PUBLIC-VECTORS
CONF-INSPECT-SNAPSHOTS
CONF-FUZZ-NO-PANIC
```

Rows to add as this roadmap is adopted:

```text
AV2-4.11.3-UVLC
AV2-4.11.5-LE
AV2-4.11.8-NS
AV2-5.2.1-OBU-DISPATCH
AV2-5.4.1-SEQUENCE-HEADER-GENERAL
AV2-5.4.2-SEQUENCE-TILE-CONFIG
AV2-5.4.3-SEQUENCE-PARTITION-CONFIG
AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG
AV2-5.4.5-SEQUENCE-INTRA-CONFIG
AV2-5.4.6-SEQUENCE-INTER-CONFIG
AV2-5.4.7-SEQUENCE-SCC-CONFIG
AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG
AV2-5.4.9-SEGMENT-INFO
AV2-5.4.10-SEQUENCE-FILTER-CONFIG
AV2-5.4.11-USER-QM
AV2-5.4.12-TIMING-INFO
AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO
AV2-6.4-SEQUENCE-HEADER-SEMANTICS
AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS
AV2-7.3.7-TEMPORAL-UNIT-ORDER
AV2-7.3.8-HLS-AVAILABILITY
```

More child rows are expected for HLS, metadata, frame header, tile group, Annex A, and Annex E as implementation starts.

## Acceptance criteria

Documentation acceptance:

```bash
cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md
cargo xtask check-feature-status
cargo xtask spec-coverage
cargo xtask ci
```

First implementation acceptance after this roadmap:

```bash
cargo test -p splot-core bitio
cargo test -p splot-core sequence_header
cargo test -p splot-validate sequence_header
cargo test -p splot-cli
cargo xtask check-feature-status
cargo xtask ci
```

## User-visible behavior after full roadmap completion

Eventually, `splot validate` should be able to distinguish:

- malformed Annex B/LEB128/OBU header streams;
- malformed payload syntax for every implemented OBU type;
- illegal sequence headers and illegal activated sequence state;
- illegal OBU ordering and HLS availability;
- illegal frame/tile syntax once corresponding child features land;
- optional AVM/vector conformance failures.

Until then, partial validation must be honest: unparsed payloads should be reported as such and matrix status must remain `partial` or `todo`.
