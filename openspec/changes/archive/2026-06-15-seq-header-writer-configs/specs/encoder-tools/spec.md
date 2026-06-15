# encoder-tools delta: seq-header-writer-configs

## ADDED Requirements

### Requirement: sequence-header config-cascade writers

`splot-core` SHALL provide writers that are the exact inverse of the § 5.4.3–5.4.8 sequence
config parsers (partition, segment, intra, inter, scc, transform/quant/entropy) and the
§ 5.4.9 `seg_info` parser. For every model the writer accepts, reparsing the written bits with
the corresponding parser SHALL yield the original (`parse(write(x)) == x`). The writers SHALL
be additive (no parser/model/parser-error changes) and SHALL never panic: a model the parser
could not have produced SHALL be rejected with a typed writer error before any bit is written.

#### Scenario: each config round-trips across every branch

- **WHEN** a parsed config is written with the same gating inputs and the bits are reparsed
- **THEN** the reparsed config SHALL equal the original, across every conditional branch.

#### Scenario: the composite rejects a bad nested seg_info before any bit

- **WHEN** the segment config carries a `seg_info` body the parser could not have produced
- **THEN** the writer SHALL reject it before writing any bit (the leading segment flags
  included), leaving the writer buffer unchanged.

#### Scenario: a non-canonical or out-of-range field is rejected before any bit

- **WHEN** any config field exceeds its bit width, lies outside its descriptor domain, or is a
  derived/inferred value the parser would re-derive differently
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.
