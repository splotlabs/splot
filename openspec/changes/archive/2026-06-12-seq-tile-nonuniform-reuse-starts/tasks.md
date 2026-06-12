# Tasks: sequence tile start arrays for non-uniform reuse

## 1. Bookkeeping

- [x] 1.1 Matrix `openspec_change` on the two rows; register in
  `openspec/changes/README.md`; re-read § 5.4.2 (05 mirror ~640-660) and
  § 5.18.7.4 (~6440-6480, uniform_spacing ~6750-6765) verbatim.

## 2. Implementation

- [x] 2.1 Persist `SeqSbColStarts` / `SeqSbRowStarts` at § 5.4.2 parse time.
- [x] 2.2 Wire into the § 5.18.7.4 reuse input; delete the non-uniform
  reuse `Unimplemented` stop and the `tiling.rs:37` TODO.

## 3. Docs

- [x] 3.1 Matrix rows advance with proof; generated docs regenerated.

## 4. Verification

- [x] 4.1 Positive/negative/EOF tests; uniform-path regression.
- [x] 4.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 4.3 `cargo xtask ci` (bare, exit checked) passes.
