# encoder-tools delta: frame-header-writer-size-config

## ADDED Requirements

### Requirement: frame-header size and configuration writers

`splot-core` SHALL provide writers that are the exact inverse of the § 5.18.4.1
`frame_size()` parser and the § 5.18.3.3 `screen_content_params()` / § 5.18.3.4
`intrabc_params()` parsers, on the intra control-region path. For every model the writer
accepts, reparsing the written bits with the corresponding parser SHALL yield the original
(`parse(write(x)) == x`), byte-exactly. The writers SHALL never panic: a model the parser
could not have produced SHALL be rejected with a typed writer error before any bit is written.

To make the `intrabc_params()` / `screen_content_params()` round-trip byte-exact rather than
merely semantic, the model and parser MAY surface the bits the modeled decode path otherwise
discards (a maintainer-approved exception to the additive / read-only-parser rule); the
surfacing SHALL NOT change the bits read (`consumed_bits` is unchanged) and SHALL preserve the
existing parser outputs.

#### Scenario: frame_size round-trips on the override and default paths

- **WHEN** a parsed `frame_size()` is written with the same gating inputs and reparsed
- **THEN** the reparsed size SHALL equal the original, for both the explicit `f(n)` override
  path and the non-override default path (which writes no bits).

#### Scenario: screen-content and intrabc params round-trip byte-exactly

- **WHEN** a parsed `screen_content_params()` / `intrabc_params()` is written with the same
  gating inputs and reparsed
- **THEN** the reparsed structure SHALL equal the original across every conditional branch
  (the `SELECT`-gated SCC/MV flags and the `frame_is_intra` / `allow_frame_max_bvp_drl_bits`
  gated intrabc fields).

#### Scenario: a non-encodable or inferred-mismatch field is rejected before any bit

- **WHEN** an overridden dimension overflows its `f(n)` field, a non-override size disagrees
  with the inferred default, an inferred SCC/MV flag disagrees with the sequence force value,
  an intrabc `Option`'s presence disagrees with its gate, or `max_bvp_drl_bits_minus_1` is
  outside the `ns(2)` domain
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.
