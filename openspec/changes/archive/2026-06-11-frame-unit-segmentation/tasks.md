# Tasks: coded-frame-unit segmentation

## 1. Bookkeeping

- [x] 1.1 Matrix `openspec_change` on the touched rows; register in
  `openspec/changes/README.md`; re-read § 7.3.3–§ 7.3.5 (07 mirror
  367–510), § 7.3.8.10, § 6.16.5/§ 6.16.6 verbatim.

## 2. Segmentation model

- [x] 2.1 Per-(xlayer, mlayer, tlayer) frame-unit state machine with the
  region order, output/non-output classification from parsed output
  flags, Unknown routing for unsupported parse stops, PADDING
  transparency, TU-end/stream-end resolution consistent with the
  established per-TU attribution.

## 3. Presence-order diagnostics

- [x] 3.1 `frame-unit/` errors per proposal item 2 (region order, CI
  multiplicity, BRT asymmetry, prefix/suffix metadata placement,
  first-tile-group flag rule, SEF single-OBU, mixed frame types).

## 4. § 7.3.7 backlog rows and § 7.3.8.10

- [x] 4.1 `obu-order/global-hls-after-metadata-suffix`,
  `obu-order/non-global-hls-before-coded-layer` (roadmap backlog rows
  removed on landing).
- [x] 4.2 § 7.3.8.10 CI-in-first-frame-unit; § 6.16.5/§ 6.16.6
  first-coded-picture halves (resolve the context.rs TODOs).

## 5. Consumer upgrades

- [x] 5.1 metadata_lifetime NO_PERSISTENCE expiry at frame-unit granularity
  (resolve its TODO); QM/FGM duplicate windows reset at true unit
  boundaries with the SEF false-negative regression.

## 6. Docs, registry, artifacts

- [x] 6.1 Register ids; matrix rows advance with proof (honest notes per
  the no-bare-partial rule; the AV2-5.5-TEMPORAL-DELIMITER SeenFrameHeader groundwork
  note); regenerate generated docs; roadmap Phase 5 + backlog table
  updated.

## 7. Verification

- [x] 7.1 Tests per acceptance criteria.
- [x] 7.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 7.3 `cargo xtask ci` (bare, exit checked) passes.
