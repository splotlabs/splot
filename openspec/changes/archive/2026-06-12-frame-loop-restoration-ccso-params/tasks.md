# Tasks: loop-restoration and CCSO frame-header params

## 1. Bookkeeping

- [x] 1.1 Matrix `openspec_change` on the rows; re-read § 5.18.7.11
  (05 mirror :7097+) and § 5.18.7.12 (:7424+) verbatim, plus the § 5.4.x
  sequence configs that gate them and the § 5.18.2 tail call sites
  (:5303-5310).

## 2. Parsing

- [x] 2.1 lr_params() per § 5.18.7.11.
- [x] 2.2 ccso_params() per § 5.18.7.12.
- [x] 2.3 Advance the intra-path stop status; EOF inside the cluster
  preserves parsed facts; audit constructed-view arithmetic for panics.

## 3. Surfacing and docs

- [x] 3.1 inspect surfaces the new fields; OpenSpec main-spec stop-point
  requirement updated; matrix rows advance with proof; generated docs
  regenerated; roadmap updated.

## 4. Verification

- [x] 4.1 Positive/negative/EOF tests per structure, gating flags both
  ways; proptests extended.
- [x] 4.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 4.3 `cargo xtask ci` (bare, exit checked) passes.
