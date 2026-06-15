# Change: frame-header-writer-prefix

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.18.1-FRAME-HEADER-GENERAL` (advances its `write` stage `todo -> partial`)

## Why

This is the foundation slice of the **frame-header writer** (backlog #4), the inverse of
the § 5.18 frame-header parser. The frame header is the largest payload and is sliced into
PR-sized changes (#4a … #4i, intra path only). This first slice inverts the § 5.18.2
**activation prefix** (`parse_frame_header_prefix`) and establishes the
`crates/splot-core/src/write/frame_header.rs` module the per-structure config writers and the
composing `write_frame_header` build on.

## What changes

- Add `crates/splot-core/src/write/frame_header.rs`:
  - `write_frame_header_prefix` — the exact inverse of `parse_frame_header_prefix`. It writes
    `cur_mfh_id` `uvlc` (omitted, inferred `0`, for a bridge frame) and, when
    `cur_mfh_id == 0`, `seq_header_id_in_frame_header` `uvlc`. The derived `is_*` / `startCVS`
    fields carry no bits.
  - `check_frame_header_prefix_encodable` (`pub(crate)`) — validates the prefix is a model the
    § 5.18.2 parser could have produced before any bit: the `is_*` / `startCVS` fields match the
    `obu_type` derivation, a bridge frame infers `cur_mfh_id == 0`, and the
    `seq_header_id_in_frame_header` / `referenced_sequence_header_id` presence matches the
    `cur_mfh_id == 0` gate.
- Add `WriteError::NonCanonicalFrameHeader { what }` for the reject-before-write paths.
- Register the module and re-export `write_frame_header_prefix` in `write/mod.rs`.
- The module is **additive**: no parser/model/parser-error edits (the only non-test change
  outside `write/` is the new error variant).

The prefix is frame-type-agnostic, exactly like the parser: it round-trips every prefix the
parser produces (bridge, `cur_mfh_id == 0`, and `cur_mfh_id > 0`). The intra-path restriction
for the full frame header is enforced later by the composing slice.

## Validator impact

None. No new diagnostics; the validator is unchanged.

## Non-goals

- No `frame_header_core` fields, no composing `write_frame_header`, no § 5.18.1
  `NumFrameHeaderBits` / `frame_header_copy()` accounting — later #4 slices.
- No encoder rate decisions; no public `encode` CLI.

## Impact

- Crate: `crates/splot-core` (additive `write` module + one new `WriteError` variant).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (+ regenerated `docs/FEATURE-STATUS.md`).
