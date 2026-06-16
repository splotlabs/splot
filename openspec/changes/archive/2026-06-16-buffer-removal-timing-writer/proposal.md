# Change: buffer-removal-timing-writer

## Feature IDs

- `AV2-5.12-BUFFER-REMOVAL-TIMING` (write: `todo` → `done`)
- `ENC-BITSTREAM-WRITER` (one of the nine unwritten OBU-type body writers; umbrella stays `partial`)

## Why

The complete-OBU dispatch returns `WriteError::Unimplemented` for nine OBU types that the parser fully
models. `buffer_removal_timing_obu()` (§ 5.12) is the smallest of them — a one-flag fork over a tiny
syntax — so it is the natural first to move from `Unwritable` to round-trippable, exercising the
dispatch's "written body + non-extensible tail" path for a new type.

## What changes

- **Writer** (`crates/splot-core/src/write/buffer_removal_timing.rs`, new; additive, no model change):
  `write_buffer_removal_timing(writer, brt: &BufferRemovalTiming)` — the inverse of
  `parse_buffer_removal_timing` (§ 5.12):
  - `br_ops_dependent_flag` `f(1)`; then either `br_time` `rg(4)` (extended-layer form) or
    `br_ops_id` `f(4)` + `br_ops_cnt` `f(3)` + `br_ops_cnt` per-operating-point entries
    (`br_decoder_model_present_op_flag` `f(1)`, and `br_time_op` `rg(4)` when present).
  - **Reject-before-write** (scratch-writer; never panics on a constructed model): a byte-alignment
    guard; `op_times.len()` that disagrees with `br_ops_cnt`; an `index` that disagrees with the
    parser's loop counter `i`; a `br_time_op` presence that disagrees with `decoder_model_present`
    (the gated-field rule); field-width / `rg` range rejects from the primitives (a `br_time` whose
    `rg(4)` quotient is `≥ 32` is parser-unproducible and rejected, not written as a huge unary run).
- **Dispatch** (`crates/splot-core/src/write/dispatch.rs`): route `ParsedObu::BufferRemovalTiming`
  to the new writer (then the generic non-extensible tail) instead of `Unimplemented`; it carries no
  passthrough, so a non-empty passthrough is rejected. The other eight types stay `Unimplemented`.
- **Error** (`write/error.rs`): add `WriteError::NonCanonicalBufferRemovalTiming { what }`
  (the per-family pattern, like `NonCanonicalMetadata`).

## Validator impact

None.

## Non-goals

- No writers for the other eight unwritten OBU types (each its own slice).
- No model change; no public `encode` command.

## Impact

- Crate: `crates/splot-core` (additive `write::buffer_removal_timing` + one `WriteError` variant +
  the dispatch arm).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (`AV2-5.12-BUFFER-REMOVAL-TIMING` write `done` +
  `ENC-BITSTREAM-WRITER` note) + regenerated `docs/FEATURE-STATUS.md`.
