# Tasks: intra frame-header tail completion

## 1. Bookkeeping

- [x] 1.1 Confirm matrix row ids (esp. the film-grain frame-state row);
  `openspec_change` set; re-read § 5.18.8.1 (05 mirror :7634+),
  § 5.18.8.2 (:7671+), § 5.18.8.3 (:7739+), § 5.18.9.1 intra arm
  (:7776+), § 5.18.10.1 (:8163+), § 5.18.10.2, and the § 5.18.2 tail
  call sites verbatim. (Film-grain row id is
  `AV2-5.18.10-FILM-GRAIN-STRUCTURES`. Confirmed `film_grain_config()`
  calls `load_grain_model()` which reads no bits per § 6.17.10.1, so no
  in-band `film_grain_model()` is parsed in the frame-header path.)

## 2. Parsing

- [x] 2.1 read_tx_mode (CodedLossless gate), frame_reference_mode (intra
  no-bit), skip_mode_params, allow_bawp/allow_warpmv inferences,
  reduced_tx_set. (`crates/splot-core/src/headers/frame/tail.rs`.)
- [x] 2.2 global_motion_params intra arm (no-bit, `use_global_motion = false`).
- [x] 2.3 film_grain_config (apply_grain/fgm_id/grain_seed; load_grain_model
  reads no bits — the § 5.14 model parser is intentionally not invoked).
- [x] 2.4 SEF path completes (ShowExistingFrameComplete); intra terminal
  status = IntraHeaderComplete; EOF preserves facts (StoppedInsideIntraTail
  for the intra tail, CoreFieldsOnly for SEF film-grain truncation);
  arithmetic audited (widened shifts; no subtraction in the new tail).

## 3. Surfacing and docs

- [x] 3.1 inspect surfaces the new fields (`intra_tail`, `sef_film_grain`,
  `immediate_output_frame`, `implicit_output_frame`); trailing-bits
  decidability recorded — NOT yet decidable (the frame header is followed by
  the rest of `tile_group_obu()` § 5.19; the `bru_inactive` `trailing_bits()`
  tail needs NumFrameHeaderBits accounting, the next backlog change), named
  as a residual on AV2-5.18.2-FRAME-HEADER-INFO; main-spec stale stop-point
  requirements edited (bitstream + validator spec.md); matrix rows advanced
  with proof; FEATURE-STATUS.md / SPEC-COVERAGE.md regenerated; roadmap
  updated.

## 4. Verification

- [x] 4.1 Positive/negative/EOF per structure; SEF completion (with and
  without grain); proptests extended (`parse_intra_tail_never_panics`).
- [x] 4.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 4.3 `cargo xtask ci` (bare, exit checked) passes.
