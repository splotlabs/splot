# Tasks

## Primitive
- [x] `bit_writer.rs`: `write_tu(value, mx)` — the §4.11.9 truncated-unary inverse (reject
      `value > mx` before any bit); unit test + round-trip vs the reader.

## Writers (additive — no model change)
- [x] `write/frame_restoration.rs`: `write_lr_params` and `write_ccso_params`, each with an
      up-front `check_*_encodable` (reject-before-write, `bit_len() == 0` on every reject).
- [x] LR: reject `frame_filters_on == true` (the Wiener-bank hard residual); reverse the
      `tool_index ns(n)` over the enabled-tools table and the `LoopRestorationSize` size-shift
      flags; validate the disabled/default and `uses_lr` derivations.
- [x] CCSO: byte-exact incl. the `ccso_offset_idx tu(7)` loop (length re-derived from
      `maxEdgeInterval² * maxBand`); the single-picture / frame-flag / bo_only / quant-step
      inferences.
- [x] Drift-proof: extract the `indexToTool` table into a `pub(crate)` helper the parser also
      calls; expose `RESTORATION_TILESIZE_MAX` / `default_restoration_size` / the CCSO quant-step
      derivation `pub(crate)`. Register + re-export in `write/mod.rs`. No `WriteError` variant.

## Tests and proof
- [x] Round-trip tests across every branch (LR: disabled, all-NONE, real tool + size signaling
      for each SbSize/shift, luma/chroma/both, num_planes 1/3; CCSO: disabled, frame_flag false,
      single-picture, all-off, bo_only, full-arm, quant-step-0 suppressed, multi-offset); one
      reject per `NonCanonicalFrameHeader` path INCLUDING the LR `frame_filters_on` hard residual
      and the unreachable-shift rejects (`bit_len() == 0`). A round-trip property test per parser
      + a never-panics-on-constructed-models proptest per writer; a `write_tu` round-trip.

## Matrix and docs
- [x] Advance the `lr_params()`/`ccso_params()` `write` portion on
      `AV2-5.18.7-SEGMENTATION-TILING` (stays `partial` — inter paths + Wiener bank remain).
      Regenerate `docs/FEATURE-STATUS.md` if a status field changes.

## Checks
- [x] `cargo xtask ci` and `openspec validate frame-header-writer-restoration-ccso --strict`
