# Change: seq-header-writer-general

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.4.1-SEQUENCE-HEADER-GENERAL` (advances its `write` stage `todo -> done`)
- `AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO` (advances its `write` stage `todo -> done`)

## Why

The sequence header is the first full payload structure; it is large enough to ship
in slices. This change lands the **general fields** (§ 5.4.1) and **decoder-model
info** (§ 5.4.13) writers — the inverse of `parse_sequence_header_general` and
`parse_sequence_decoder_model_info` — on top of the merged `BitWriter` primitives and
OBU writers. The config children (§ 5.4.2–5.4.10) and the table-driven `tile_params()`
follow in `seq-header-writer-configs` and `seq-header-writer-tile`.

The hard correctness piece is the **dependency maps**: the model stores the *derived*
`MLayerDependencyMap`/`TLayerDependencyMap` plus present-flags, not the raw signaled
bits, so the writer must re-derive the signaled bits exactly as the parser would and
reject any map the parser could never have produced. Isolating it in its own PR keeps
the review focused.

## What changes

- Add `crates/splot-core/src/write/seq_header.rs`: public `write_sequence_header_general`,
  `write_dependency_maps`, `write_cropping_window`, `write_sequence_decoder_model_info`,
  each with an up-front `check_*_encodable` validator (reject before any bit is written).
- Add `WriteError::NonCanonicalSequenceValue { what }` for a derived/inferred model value
  the § 5.4 parser could not have produced. The module stays **additive** — no parser,
  model, or parser-error edits.

## Validator impact

None. No new diagnostics; the validator is unchanged. The writers are reachable only
from library code and tests.

## Non-goals

- No config writers (§ 5.4.2–5.4.10) and no `tile_params()` — later sub-changes.
- No top-level `write_sequence_header` yet (it composes the config writers, which do not
  exist; tested instead via the public `parse_sequence_header_general` oracle).
- No `write_timing_info` (it is reached from the § 5.15 content-interpretation OBU, a
  later writer); no encoder rate decisions; no public `encode` CLI.

## Impact

- Crate: `crates/splot-core` (additive `write` module only).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (+ regenerated `docs/FEATURE-STATUS.md`).
- The implementation matrix remains the source of truth for status.
