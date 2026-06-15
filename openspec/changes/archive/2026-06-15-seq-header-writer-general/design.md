# Design: seq-header-writer-general

## Context

`parse_sequence_header_general` (AV2 § 5.4.1) reads the general sequence-header fields
through the dependency maps, stopping before the first child config; the configs are
parsed by `parse_sequence_header`. The general fields include `uvlc`/`f(n)` scalars,
several conditionally-present fields, an inline `seq_decoder_model_info()` (§ 5.4.13)
when signalled, and the `mlayer`/`tlayer` dependency maps. The writer must invert all of
this. The contract is the round-trip: `parse_sequence_header_general(write(g)) == g` for
every model the writer accepts.

## Decision D1 — public sub-config writers, tested via the parser oracle

The top-level `write_sequence_header` cannot exist until the config writers do, so this
change ships the four general-fields writers as **public** functions (mirroring how the
parser exposes `parse_sequence_header_general` publicly) and tests each by round-tripping
through the public parser. No incomplete `write_sequence_header` is shipped.

## Decision D2 — reject before write, including derived/inferred state

Following the established writer philosophy (reject exactly the values the parser could
never produce, so round-trip holds for everything accepted), each writer validates fully
up front in a `check_*_encodable` helper before any bit is written (tested with
`bit_len() == 0` on every reject path). Rejections include: field-width overflow
(`ValueTooWide`); `uvlc`/`ns` domain (`ValueOutOfRange`); unaligned writer
(`WriterNotByteAligned`); and the new `NonCanonicalSequenceValue` for a derived/inferred
value the parser would re-derive differently — a `seq_tier` whose conditional gate is
false, a single-picture header carrying a non-inferred constant, an `Option` field whose
presence disagrees with its gating flag, a cropping window non-default while its flag is
clear, or a dependency map not reproducible from its present-flags.

## Decision D3 — the dependency-map inverse

The model stores the *derived* maps + present-flags, not the raw bits. The writer
re-derives the signaled bits in the parser's exact loop order (mlayer: `curr 1..=max`,
`ref (0..=curr).rev()`; tlayer: `m 0..=max_mlayer`, `curr 1..=max_tlayer`,
`ref (0..=curr).rev()`, emitting a bit only when `multi || m == 0`), and the `multi` flag
only when `max_mlayer_id > 0`. `check_dependency_maps_encodable` rejects any map that is
not the § 5.4.1 default fill plus reproducible signaled overrides: a present-flag-clear
map differing from the default, an unsignaled entry (row 0, strict upper triangle, rows
beyond max) differing from the default, a non-`multi` map whose layers `>0` are not a
verbatim copy of layer 0, or `multi` set while `max_mlayer_id == 0`. This is the spine of
the change and the reason it is isolated.

## Round-trip test plan

- Semantic round-trip property test over parser-reachable models (every branch:
  single-picture, mono/non-mono, the `seq_tier` gate, dependency maps with `multi`/
  non-`multi` and row-0 replication, cropping present/absent, decoder-model present/
  absent, max layer ids), asserting `parse(write(g)) == g` and byte-stability.
- Byte-exact unit test on a hand-built fixture from the parser tests.
- One rejection test per `WriteError` path, each asserting `bit_len() == 0`.
- A never-panics property test over arbitrary bytes.

## Byte-exact scope

Byte-exact round-trip holds for the canonical general-fields encoding; the general fields
end mid-byte (the configs continue in the same byte), so byte-exactness is asserted at the
whole-byte-vector level on a standalone general-fields fixture, and the universal contract
is the semantic round-trip. Full-header byte-exactness lands with the composing
`write_sequence_header` in a later sub-change.
