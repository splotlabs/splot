# Change: frame-header-writer-size-config

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.18.4-FRAME-SIZE`, `AV2-5.18.3-FRAME-CONFIGURATION`
  (each advances its `write` stage `todo -> partial`)

## Why

Second slice (#4b) of the frame-header writer (intra path). It inverts the § 5.18.4
`frame_size()` and § 5.18.3 `screen_content_params()` / `intrabc_params()` structures the
intra control region reaches.

`intrabc_params()` (and `screen_content_params()`'s `force_integer_mv`) read several bits the
intra decode process derives nothing from and the model previously discarded. The maintainer
chose **full byte-exact** round-trip over canonicalization, so this change first **extends the
model and parser** to surface those bits — a deliberate, approved exception to the writer
mission's additive / read-only-parser constraint — then writes the faithful inverse.

## What changes

- **Model + parser surfacing** (the approved exception):
  - `headers/frame/config.rs`: add `IntrabcParams` + `parse_intrabc_params_full`, surfacing
    `allow_global_intrabc` / `allow_local_intrabc` / `change_bvp_drl` /
    `max_bvp_drl_bits_minus_1` (`Some` exactly when the bit was present). `parse_intrabc_params`
    becomes a thin `bool` wrapper (the inter caller is unchanged); remove the now-unused
    `parse_screen_content_params` `bool` wrapper.
  - `headers/frame/info.rs`: `FrameHeaderCore` gains `force_integer_mv` and `intrabc`; the intra
    path populates them via the `_full` parsers. `consumed_bits` is unchanged (identical bits
    read), and `allow_screen_content_tools` / `allow_intrabc` remain for existing consumers.
  - `headers/frame/mod.rs`: re-export `IntrabcParams` (and `pub(crate)` re-exports of the
    `_full` parsers + `parse_frame_size` for the writer).
- **Writers** (`crates/splot-core/src/write/frame_config.rs`):
  - `write_frame_size` — the § 5.18.4.1 override `f(n)` width/height, or no bits on the
    non-override default path.
  - `write_screen_content_params` — the § 5.18.3.3 `SELECT`-gated flags.
  - `write_intrabc_params` — the § 5.18.3.4 fields, now byte-exact via the surfaced
    `IntrabcParams`.
  - Each validates the whole structure before any bit (reject-before-write).

## Validator impact

None. No new diagnostics; the validator is unchanged.

## Non-goals

- No `frame_size_with_refs/with_bridge` (§ 5.18.4.2/.3) or `frame_opfl_refine_type`
  (§ 5.18.3.2) — inter paths.
- No composing `write_frame_header` — a later #4 slice.

## Impact

- Crate: `crates/splot-core` (the approved model/parser surfacing + the additive `write`
  module). No new `WriteError` variant (the existing variants suffice).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (+ regenerated `docs/FEATURE-STATUS.md`), and a
  documented `info.rs` source-line allowance bump for the added `FrameHeaderCore` fields.
