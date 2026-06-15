# Tasks

## Writers (additive — no model change)
- [x] `write/frame_tail.rs`: `write_tx_mode`, `write_film_grain_config`, `write_intra_tail`,
      each with an up-front `check_*_encodable` (reject-before-write, `bit_len() == 0`).
- [x] `write_intra_tail` re-validates `tx_mode` + the film-grain model BEFORE the first bit, so
      a film-grain reject cannot leave a partial buffer. Register + re-export in `write/mod.rs`.
      No model field / `WriteError` variant added.

## Tests and proof
- [x] Round-trip tests across every branch (tx_mode lossless/largest/select; film_grain
      gated-off / not-output / single-picture-inferred / coded-apply true+false; intra_tail
      lossless / non-lossless / grain-absent); one reject test per `NonCanonicalFrameHeader`
      path (`bit_len() == 0`), incl. the partial-buffer-guard (a valid prefix with a bad
      film-grain field). A round-trip property test per parser + a never-panics proptest.

## Matrix and docs
- [x] Advance `write` `done` on `AV2-5.18.8-TRANSFORM-CODING-MODES` and
      `AV2-5.18.10-FILM-GRAIN-STRUCTURES` (intra), note the `AV2-5.18.9-GLOBAL-MOTION` no-bit
      intra arm (row stays `partial`). Regenerate `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate frame-header-writer-intra-tail --strict`
