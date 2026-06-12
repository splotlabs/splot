# Tasks: § 5.19 tile-group structure completion

## 1. Bookkeeping

- [x] 1.1 Matrix rows confirmed; `openspec_change` set; re-read § 5.19
  (05 mirror :8431-8530) verbatim, the § 6 tg_start/tg_end semantics,
  the § 5.18.2 intra inferences for use_bru/bru_inactive, and
  byte_alignment (:284).

## 2. Implementation

- [x] 2.1 Parse the remainder on intra-complete paths; record the
  headerBytes/payload boundary.
- [x] 2.2 tg-range diagnostics per the § 6 clauses found.
- [x] 2.3 BRU arms: intra derivations or honest stops; EOF/truncation
  per the established pattern.

## 3. Surfacing and docs

- [x] 3.1 inspect surfaces the structure; matrix proof; named residuals
  (§ 5.20 payload, inter BRU); generated docs; roadmap.

## 4. Verification

- [x] 4.1 Positive/negative/EOF per element; both tg modes; proptests.
- [x] 4.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 4.3 `cargo xtask ci` (bare, exit checked) passes.
