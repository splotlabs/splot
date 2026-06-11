# Proposal: Annex A profile/level/tier static-constraint skeleton

## Feature IDs

- `AV2-A-PROFILES` (todo → partial)
- `AV2-A-LEVELS-TIERS` (todo → partial)
- `AV2-5.11.2-OPS-SEQ-PTL-INFO` (OPS-signaled level/tier value-space; note-level)

## Why

Zero Annex A validation exists anywhere in `splot-validate` — both Annex A
matrix rows are `validate = todo` while a substantial static subset is
checkable today from already-parsed state: the activated sequence header
carries `seq_profile_idc`, `chroma_format_idc`, `bit_depth_idc`, `seq_tier`,
`seq_level_idx`; the intra frame-header path yields `FrameWidth`/`FrameHeight`
and the tile layout; the CMVS/distinct-layer trackers count extended and
embedded layers. A validator that ignores profiles and levels accepts streams
no conformant decoder is required to decode.

## What Changes

New `annex-a/` diagnostics (namespace already reserved in the registry), all
grounded in `docs/spec/av2/1.0.0/annex-a-profiles-levels-and-tiers.md`:

1. **Profile checks (Table A.1, mirror lines 59–91)** on sequence-header
   activation:
   - `annex-a/profile-reserved` (error): `seq_profile_idc` in 5–30.
   - `annex-a/profile-chroma-format-mismatch` (error): `chroma_format_idc`
     outside the profile's allowed set (0–2: 4:0:0/4:2:0; 3: +4:2:2;
     4: +4:4:4; 31: unconstrained).
   - `annex-a/profile-bit-depth-mismatch` (error): `bit_depth_idc` not 0/1
     for profiles 0–4 (31: unconstrained).
2. **Level/tier value-space (Tables A.7/A.9, lines 269–438)**:
   - `annex-a/level-reserved` (error): `seq_level_idx` in 22–30 (today this
     is only a bounded parse stop in the tile-params path). Also applied to
     `ops_level_idx` (Annex A applies constraints per sub-bitstream using
     OPS-derived values, lines 443–451).
   - `annex-a/high-tier-below-4-0` (warning): `seq_tier == 1` with
     `seq_level_idx < 4`. Warning, not error: the only spec statement is the
     Table A.9 NOTE ("seq_tier equal to 1 can only be signaled for level 4.0
     and above", lines 436–437) plus the undefined HighMbps/HighCR cells —
     a NOTE is informative, so error severity would overclaim.
3. **Static level-limit checks (Table A.8 + the §A.4 conformance block,
   lines 615–629)** on the parsed intra frame path, skipped entirely when
   `seq_level_idx == 31` (line 659):
   - `annex-a/frame-size-exceeds-level` (error): `FrameWidth * FrameHeight >
     MaxPicSize`, `FrameWidth > MaxHSize`, or `FrameHeight > MaxVSize`.
   - `annex-a/frame-size-below-minimum` (error): `FrameWidth < 16` or
     `FrameHeight < 16`.
   - `annex-a/tile-count-exceeds-level` (error): `NumTiles > MaxTiles` or
     `TileCols > MaxTileCols` (Table A.9 columns).
   Table data: a new `splot-validate` module transcribes the needed columns
   of Tables A.7/A.8/A.9 (LevelIdx 0–21 → MaxPicSize, MaxHSize/MaxVSize,
   MaxTiles, MaxTileCols) verbatim from the mirror with per-row citations.
   Rate columns are NOT transcribed (Annex E change owns them).
4. **Table A.4 IOP presence requirements (lines 173–201)** at CVS/CMVS scope,
   for profiles 0–4 (IOP from Table A.1; layer counting per the Table A.3
   definitions, lines 144–161, from existing distinct-xlayer/mlayer state):
   - `annex-a/msdo-prohibited-for-iop` (error), `annex-a/msdo-required-for-iop`
     (error), `annex-a/lcr-required-for-iop` (error) — exact per-row semantics
     in design.md, including the IOP2 either/or rows.

## Non-goals

- Rate-based constraints (MaxDisplayRate/MaxDecodeRate/MaxHeaderRate,
  CompressedSize, FrameSymbolCount, MaxLevelRefFrames, buffer model) — the
  `annex-e-decoder-model-schedule` backlog item; needs frame-unit timing.
- Per-tile TileWidth/TileHeight scaling-factor constraints (lines 623–627):
  the §5.18.7.2 syntax already bounds tile sizing via the same scaling tables
  at parse; re-deriving the per-tile Annex A inequalities adds no detection
  power until non-uniform tile layouts parse fully.
- `MultiStreamDecoderMode == 1` substream level scaling (lines 456–523):
  needs MSDO-state plumbing; recorded as a spec TODO on
  `AV2-A-LEVELS-TIERS`.
- Configurable-profile (31) constraint derivation and Table A.5/A.6
  multi-sequence configuration agreement.
- Still-picture-gated constraints (lines 631–647).

## Acceptance criteria

- [ ] Every transcribed table value diff-checks against the mirror (review
  step); no value from memory.
- [ ] Every diagnostic cites Annex A and the mirror path; positive, negative,
  and boundary tests per check (level 31 disables; profile 31 unconstrained;
  exact limit values pass, limit+1 fails).
- [ ] Matrix rows advance with proof; `check-feature-status`,
  `check-diagnostic-registry`, and `cargo xtask ci` pass.
- [ ] splot-validate line coverage stays ≥ 90% (CI gate).
