# Change: frame-header-writer-intra-tail

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.18.8-TRANSFORM-CODING-MODES` (advances `read_tx_mode()`'s `write` stage)
- `AV2-5.18.10-FILM-GRAIN-STRUCTURES` (advances `film_grain_config()`'s `write` stage on the
  intra path)
- `AV2-5.18.9-GLOBAL-MOTION` (the intra no-bit `global_motion_params()` arm; the row stays
  `partial` — the per-reference warp loop is inter-only and unmodeled)

## Why

Eighth slice (#4h) of the frame-header writer (intra path). It inverts the § 5.18.2 intra
**tail**: `read_tx_mode()` (§ 5.18.8.1), the no-bit intra inferences, `reduced_tx_set`, the
no-bit intra arm of `global_motion_params()` (§ 5.18.9.1), and `film_grain_config()`
(§ 5.18.10.1). This is additive — no model change; the tail is a sequence of direct field
writes plus inferred no-bit fields that are re-derived and rejected on mismatch.

## What changes

- **Writers** (`crates/splot-core/src/write/frame_tail.rs`): `write_tx_mode`,
  `write_film_grain_config`, `write_intra_tail`, each validating the whole model up front
  (`check_*_encodable`, reject-before-write; `bit_len() == 0` on every reject).
- `write_tx_mode`: `ONLY_4X4` is inferred (no bit) for a lossless frame; otherwise
  `tx_mode_select` `f(1)` (`ONLY_4X4` is unreachable and rejected).
- `write_film_grain_config`: the three-way `apply_grain` gate (forced-`0` when grain is absent
  or the frame is not output; inferred-`1` for a single-picture header; else coded `f(1)`), then
  `fgm_id` `f(3)` and `grain_seed` `f(16)` when set.
- `write_intra_tail`: the five no-bit intra inferences (`reference_select` /
  `skip_mode_present` / `allow_bawp` / `allow_warpmv_mode` / `use_global_motion`) are validated
  `false` and never coded; `reduced_tx_set` `f(2)` is written; `read_tx_mode()` and
  `film_grain_config()` are re-validated before the first bit so a film-grain reject cannot
  leave a partial buffer.
- **No model field and no new `WriteError` variant** (reuses `NonCanonicalFrameHeader`).

## Validator impact

None. No new diagnostics.

## Non-goals

- No inter-path `global_motion_params()` per-reference warp decode (unmodeled; the intra arm is
  no-bit).
- No composing `write_frame_header` — that is the final #4i slice.

## Impact

- Crate: `crates/splot-core` (additive `write` module).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml`.
