# encoder-tools Specification

## Purpose

Encoder strategy and tooling that is not a direct AV2 syntax element: the toy intra
path, rate control, and the bitstream writer foundation. These require design before
code and produce only deliberately simple, writer-validated streams at first.

Tracked by Feature IDs: `ENC-BITSTREAM-WRITER`, `ENC-INTRA-TOY-V0`,
`ENC-RATE-CONTROL-V0`.

**Status: planned.** No part of this capability is implemented yet (those rows are
`todo`/proposed in the matrix). The requirements below are the accepted *target*
contract; implementation is proposed in `openspec/changes/add-bitstream-writer` and
`openspec/changes/toy-intra-encoder-v0`.

## Requirements

### Requirement: writer symmetry

When implemented, the bitstream writer SHALL be symmetric with the parsers: anything
the writer emits SHALL parse back to an equal structure (round-trip tests) before
the writer stage is marked `done` in the matrix.

#### Scenario: round-trip

- **WHEN** the writer emits a syntax element
- **THEN** parsing the bytes back yields an equal structure

### Requirement: legal-stream-only encoding

When implemented, the toy encoder SHALL emit only syntax that is implemented in the
writer and accepted by `splot validate`. It SHALL NOT fabricate AV2 syntax.

#### Scenario: toy intra output

- **WHEN** a single intra frame is encoded by the toy path
- **THEN** the output validates clean under `splot validate`
