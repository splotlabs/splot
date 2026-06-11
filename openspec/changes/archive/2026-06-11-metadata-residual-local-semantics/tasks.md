# Tasks: metadata timecode and frame-hash residuals

## 1. Bookkeeping

- [x] 1.1 Matrix `openspec_change` on the two rows; register in
  `openspec/changes/README.md`; re-read § 6.16.7/§ 6.16.13 mirror text and
  the parsed timecode model.

## 2. Inference-presence rules

- [x] 2.1 Track per-field presence of the previous clock-timestamp set in
  decoding order (scope chosen per existing metadata-lifetime conventions,
  documented); emit `metadata/timecode-inferred-without-previous` (error,
  § 6.16.7) per absent-field-without-previous, message naming the field.

## 3. Timing-gated bound

- [x] 3.1 Determine whether `ci_timing_info_present_flag`-gated state
  (time_scale / TicksPerPicture) is parsed and trackable; implement
  `metadata/timecode-n-frames-exceeds-rate` (error) with boundary tests, or
  defer with a precise matrix-note blocker. DECISION: decidable. The
  `ci_timing_info_present_flag` is the content-interpretation OBU's flag
  (annex-e line 293), and time_scale / num_units_in_display_tick /
  num_ticks_per_picture_minus_1 are parsed into `TimingInfo` and tracked in the
  `content_interpretations` store; implemented with both arrival orders and
  strict-`<` boundary tests.

## 4. Frame-hash reserved fields

- [x] 4.1 Verify § 6.16.13 reserved-field facts vs the existing parse/checks;
  add only what the mirror states and the code lacks. Added
  `metadata/decoded-frame-hash-reserved-nonzero` (warning) for the `reserved`
  bit's "shall be set to 0 and ignored by decoders"; hash_type's value-space
  note carries no "shall", so no diagnostic.

## 5. Docs, registry, artifacts

- [x] 5.1 Register ids; matrix advances with proof (timecode note lists what
  landed and the named output-order blocker); regenerate generated docs.

## 6. Verification

- [x] 6.1 Tests per acceptance criteria.
- [x] 6.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 6.3 `cargo xtask ci` (bare, exit checked) passes.
