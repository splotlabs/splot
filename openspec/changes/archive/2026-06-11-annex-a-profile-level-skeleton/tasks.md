# Tasks: Annex A profile/level/tier static-constraint skeleton

## 1. Pre-implementation bookkeeping

- [x] 1.1 docs/IMPLEMENTATION-MATRIX.toml: set `openspec_change =
  "annex-a-profile-level-skeleton"` on `AV2-A-PROFILES` and
  `AV2-A-LEVELS-TIERS`.
- [x] 1.2 Register the change in `openspec/changes/README.md`.
- [x] 1.3 Add the `annex-a/` ids to `docs/VALIDATOR-DIAGNOSTICS.md` as they
  land (registry gate enforces exact agreement) and confirm the namespace
  rules in `docs/FEATURE-TRACKING.md` § 12 cover `annex-a/`. (Added `annex-a/`
  to `DIAGNOSTIC_PREFIXES`; § 12 points to that allowlist + the registry.)

## 2. Table and profile data module

- [x] 2.1 `crates/splot-validate/src/annex_a.rs`: transcribe Table A.7
  (LevelIdx 0–21 valid, 22–30 reserved, 31 max-parameters), Table A.8
  (MaxPicSize, MaxHSize/MaxVSize), Table A.9 (MaxTiles, MaxTileCols), and
  Table A.1 (profile → allowed chroma formats, bit depths, IOP) **verbatim
  from the mirror**, with per-table mirror citations. No rate columns.
- [x] 2.2 Unit test asserting spot values quoted from the mirror in the test
  body (per design.md Testing).

## 3. Profile and value-space checks (sequence activation)

- [x] 3.1 `annex-a/profile-reserved` (error, § A.2 Table A.1).
- [x] 3.2 `annex-a/profile-chroma-format-mismatch` (error, § A.2 Table A.1);
  profile-31 skip.
- [x] 3.3 `annex-a/profile-bit-depth-mismatch` (error, § A.2 Table A.1);
  profile-31 skip. (Defensive: the parsed `BitDepthIdc` only models 0/1, so
  an out-of-range value is rejected at parse before activation — documented.)
- [x] 3.4 `annex-a/level-reserved` (error, § A.4 Table A.7) on the activated
  header's `seq_level_idx` and on observed `ops_level_idx` values.
- [x] 3.5 `annex-a/high-tier-below-4-0` (warning, § A.4 Table A.9 NOTE) with
  the severity rationale in the message/doc. The reachable arm is the OPS path:
  a sub-bitstream's `seq_tier`/`seq_level_idx` may be derived from the
  OPS-signaled `ops_tier_flag`/`ops_level_idx` (mirror lines 443–451), and the
  OPS PTL syntax (§ 5.11.2) carries `ops_tier_flag` unconditionally, so
  `ops_tier_flag == 1 && ops_level_idx < 4` is checked in
  `check_ops_level_tier_value_space`. The sequence-header arm is
  syntax-unreachable (`seq_tier` is only signaled for `seq_level_idx > 3`) and is
  kept as a documented defensive guard pinned by a test.

## 4. Level-limit checks (intra frame path)

- [x] 4.1 `annex-a/frame-size-exceeds-level` (error, § A.4 lines 615–620):
  MaxPicSize / MaxHSize / MaxVSize; level-31 skip.
- [x] 4.2 `annex-a/frame-size-below-minimum` (error, § A.4 lines 628–629):
  FrameWidth/FrameHeight >= 16; level-31 skip (the >=16 rule is inside the
  LevelIdx-gated block in the mirror, so it skips at level 31 like the rest —
  `level_limits` returns `None` for 31/reserved, disabling all checks).
- [x] 4.3 `annex-a/tile-count-exceeds-level` (error, § A.4 lines 621–622 +
  Table A.9): NumTiles/MaxTiles, TileCols/MaxTileCols; level-31 skip.

## 5. Table A.4 IOP presence checks (CVS scope)

> **DESCOPED before merge (PR #46).** Codex's second review pass found the
> Table A.4 IOP-presence window machinery unsound without state this skeleton
> does not model (MSDO aggregate-profile `multistream_profile_idc` state, LCR
> activation state, §7.3.6-correct per-TU window attribution). All of section 5
> was removed cleanly before merge — types (`AnnexAIopTracker`,
> `AnnexAIopWindow`, `AnnexAIopState`), the window plumbing and emit paths, the
> three diagnostics, and their tests — and re-lands with the
> `msdo-global-lcr-agreement` backlog change. The three ids moved to the Planned
> diagnostics backlog in `docs/VALIDATOR-ROADMAP.md` (the PR #46 codex threads
> record the requirements).

- [x] ~~5.1 Layer counting per Table A.3 definitions from existing trackers
  (design.md); embedded-layer count from activated headers.~~ (Descoped before
  merge — see the section note.)
- [x] ~~5.2 `annex-a/msdo-prohibited-for-iop`, `annex-a/msdo-required-for-iop`,
  `annex-a/lcr-required-for-iop` (errors, § A.2 Table A.4) with the design.md
  row semantics, including both IOP2 either/or arms; fire at CVS end;
  `ExternalHlsMode::Provided` suppression with tests.~~ (Descoped before merge —
  see the section note.)
- [x] ~~5.3 If the Table A.3 layer-budget bound (combination flag for IOP 0/1)
  is not trivially provable here, record it as `TODO(spec:
  AV2-A-LEVELS-TIERS)` instead of guessing.~~ (Descoped before merge — see the
  section note.)

## 6. Docs, registry, generated artifacts

- [x] 6.1 All new ids registered in `docs/VALIDATOR-DIAGNOSTICS.md` with spec
  sections; note that no AVM differential oracle has been run for these yet
  (avm_diff stays pending).
- [x] 6.2 Matrix rows advance with proof (tests + diagnostics);
  `docs/VALIDATOR-ROADMAP.md` Annex A mention updated; regenerate
  FEATURE-STATUS/SPEC-COVERAGE.
- [ ] 6.3 Commit content, then re-record the audit ledger and commit it
  separately (content commit → ledger regen → ledger commit). (Deferred:
  orchestrator handles commits/ledger.)

## 7. Verification

- [x] 7.1 Boundary tests pass-at-limit / fail-past-limit for every level
  limit; profile/chroma/bit-depth matrix; reserved values; level-31 and
  profile-31 skips; Table A.4 all rows; suppression cases.
- [x] 7.2 `cargo xtask check-feature-status` and `check-diagnostic-registry`
  pass.
- [ ] 7.3 `cargo xtask ci` passes end to end (run bare, check exit code).
