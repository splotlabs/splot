# Tasks: MFH frame-header state threading

## 1. Bookkeeping

- [x] 1.1 Matrix `openspec_change` on the four rows; register in
  `openspec/changes/README.md`; re-read § 5.7 (05 mirror 1700-1760),
  § 5.18.4 (:5760-5775), § 5.18.7.1 (:6260-6300), the inference rules
  (:4081/:4101) verbatim.

## 2. Threading

- [x] 2.1 Carry the parsed § 5.7 fields to the core parse's MFH input
  (widen the validator record or pass the parsed OBU view); replace the
  reserved stub; unresolvable MFH keeps Unknown routing.

## 3. Stop removals

- [x] 3.1 frame_size(): MFH default dims (with the omitted-size inference).
- [x] 3.2 segmentation_params(): the MFH-gated arms per § 5.18.7.1.
- [x] 3.3 mfh_deblocking_filter_update recorded as groundwork; residual
  note names frame-filtering-deblocking-gdf-cdef.

## 4. Surfacing and docs

- [x] 4.1 inspect surfaces newly parsed MFH-path fields; generated docs
  regenerated; matrix rows advance with proof.

## 5. Verification

- [x] 5.1 Positive/negative/EOF tests per parser change, each MFH flag
  both ways; unresolvable-MFH Unknown tests.
- [x] 5.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 5.3 `cargo xtask ci` (bare, exit checked) passes.
