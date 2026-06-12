# Tasks: § 5.18.9 inter global-motion arm

## 1. Bookkeeping

- [x] 1.1 `openspec_change` on the row; re-read § 5.18.9.1-.6 verbatim
  (05 mirror :7776+) plus the § 6.17.x semantics for the new fields and
  whatever cross-frame state the base-selection arm consumes.

## 2. Parsing

- [x] 2.1 use_global_motion + base selection (SWITCH inference, our_ref
  ns(n), the RefNumTotalRefs arm or its honest stop).
- [x] 2.2 The per-ref warp loop: read_global_param + the subexp chain
  (§ 5.18.9.3-.6), arithmetic audited.
- [x] 2.3 EOF = facts-preserving truncation; honest stops.

## 3. Validation, surfacing, docs

- [x] 3.1 Decidable § 6 diagnostics or named residuals; inspect
  surfaces the warp state; matrix proof; generated docs; roadmap.

## 4. Verification

- [x] 4.1 Per-element tests; subexp hand-computed vectors; proptests.
- [x] 4.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 4.3 `cargo xtask ci` (bare, exit checked) passes.
