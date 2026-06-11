# Design: Annex A profile/level/tier static-constraint skeleton

## Where the checks live

- New module `crates/splot-validate/src/annex_a.rs`: the transcribed level
  table (`LevelLimits { max_pic_size, max_h_size, max_v_size, max_tiles,
  max_tile_cols }` indexed by LevelIdx 0–21), the profile table (allowed
  chroma formats / bit depths / IOP per `seq_profile_idc`), and pure helper
  functions. Doc comments cite the mirror per table, with the source line
  ranges. `MaxVSize` and `MaxHSize` share one column in Table A.8 ("MaxHSize/
  MaxVSize") — model as a single field used for both bounds, with a comment
  quoting the column header so reviewers do not suspect a transposition.
- Check wiring in `crates/splot-validate/src/context.rs`:
  - Profile + level/tier value-space checks fire on **frame-confirmed
    sequence-header activation** (the same place existing `sequence-header/*`
    activation checks fire), once per activated header per CVS, not per OBU.
  - Frame-size / tile-count level checks fire on the **parsed intra
    frame-header path** next to the existing `frame_tile_info_checks`
    (context.rs ~6481), using the activated header's `seq_tier`/
    `seq_level_idx`.
  - Table A.4 IOP presence evaluation fires at **CVS end** (the existing CVS
    boundary handling), when the extended/embedded layer counts for the CVS
    are final: counting earlier would false-positive on streams whose MSDO or
    LCR arrives later in the CVS. Layer counts per Table A.3 (mirror lines
    144–161): extended layers from the existing distinct-`obu_xlayer_id`
    tracking (excluding `GLOBAL_XLAYER_ID`), preferring
    `num_streams_minus_2 + 2` when `MultiStreamDecoderMode == 1` and
    `LcrMaxNumXLayerCount` when a global LCR is activated; embedded layers
    from the max `seq_max_mlayer_cnt_minus_1 + 1` across activated headers.
- `ops_level_idx` reserved-range check fires where OPS records are observed
  (existing `ops/*` check site).

## Table A.4 row semantics (lines 178–201)

Let E = (extended layers > 1), M = (embedded layers > 1), IOP from the
activated profile (Table A.1: profile 0 → IOP0; 1, 3, 4 → IOP1; 2 → IOP2).
Presence means "an OBU of that type occurred in the CVS/CMVS".

| IOP | E | M | rule |
|---|---|---|---|
| 0 | N | – | MSDO prohibited |
| 0 | Y | – | MSDO required |
| 1 | N | N | MSDO prohibited |
| 1 | Y | N | MSDO required |
| 1 | N | Y | MSDO prohibited; local LCR required |
| 2 | N | N | MSDO prohibited |
| 2 | Y | N | MSDO **or** global LCR required (either satisfies) |
| 2 | N | Y | MSDO prohibited; LCR required (global or local) |
| 2 | Y | Y | (MSDO **and** local LCR) **or** global LCR required |

IOP1 with E∧M has no row in Table A.4 — that combination is outside IOP1's
layer budget (Table A.3 says combination flag must be 0 for IOP 0/1), so it is
not checked here; exceeding the IOP layer budget itself is interoperability
conformance that needs Table A.3 bounds — implement only what Table A.4
states, and record the Table A.3 layer-budget check as a spec TODO if not
trivially provable in this change.

## Severities

Errors throughout, except `annex-a/high-tier-below-4-0` (warning — sourced
from a NOTE, see proposal). Reserved profile (5–30) and reserved level
(22–30) are errors: Annex A.2/A.4 define the value spaces for "this version
of this specification", and a reserved value means the stream does not
conform to any defined profile/level of this version.

## Suppression and guards

- `seq_level_idx == 31`: all level-limit checks skip (mirror line 659);
  value-space checks still apply (31 is valid).
- Profile 31 (Configurable): chroma/bit-depth checks skip (Table A.1 dashes);
  reserved-profile check still applies to 5–30 only.
- `ExternalHlsMode::Provided`: Table A.4 presence requirements are suppressed
  (externally-supplied HLS makes in-band presence counting unsound), matching
  how existing `hls/*` checks handle the mode. Value-space and frame-vs-level
  checks are NOT suppressed (they do not depend on HLS availability).
- Unknown state never fires a diagnostic: no activated sequence header → no
  Annex A checks (the existing `hls/unavailable-sequence-header` already
  reports that condition).

## Testing

Synthetic OBU streams per check: pass-at-limit / fail-past-limit boundary
pairs (e.g. a level-2.0 stream with FrameWidth*FrameHeight == 147456 passes;
+1 sample fails), profile/chroma/bit-depth matrix cases, level-31 and
profile-31 skip cases, reserved values, Table A.4 row coverage including both
IOP2 either/or arms, and ExternalHlsMode suppression. Table transcription
gets a dedicated unit test asserting spot values quoted in the test from the
mirror (e.g. LevelIdx 9 → MaxPicSize 8912896, MaxTiles 64).
