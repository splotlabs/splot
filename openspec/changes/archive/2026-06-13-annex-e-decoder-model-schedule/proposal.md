# Proposal: Annex E decoder-model schedule and the rate-based Annex A constraints

## Feature IDs

- `AV2-E-DECODER-MODEL` (todo → the Annex E smoothing-buffer simulation
  and § E.7 conformance checks)
- `AV2-5.12-BUFFER-REMOVAL-TIMING` (the parsed `br_time` values become
  schedule inputs; § 6.11 RAP-relative semantics)
- `AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO` /
  `AV2-5.11.3-OPS-DECODER-MODEL-INFO` (parsed timing/delay state feeds
  the model)
- `AV2-A-LEVELS-TIERS` (the DYNAMIC half: the Annex A.4 rate
  constraints that need `Removal[]` times)
- `AV2-5.17.11-METADATA-TEMPORAL-POINT-INFO` (presentation-time
  inputs)
- `AV2-7.3-OBU-ORDERING` (the parked `brt/global-ordering-position`
  backlog row resolves with this state — the TODO at the
  `splot-validate` context BRT handler)

## Why

The decoder model is the last wholly-unmodeled normative annex with
validator-decidable arithmetic. Its prerequisites have all landed:
frame-unit segmentation (the DFG boundary authority), the complete
intra frame header, the inter control region, reference-state and
random-access models. Annex E.4-E.6 (mirror
`annex-e-decoder-model.md`) defines a deterministic arithmetic
simulation — DFG bit arrival, scheduled/actual removal, decode and
presentation timing — over already-parsed syntax
(`decoder_model_info`, per-OPS `decoder_model_info_present_flag` arms,
`buffer_removal_timing`, `temporal_point_info`). § E.7 then states
pure conformance expressions over the simulated times (E.7.1
availability + non-decreasing presentation + signaled-BRT floor,
E.7.2 `DecoderBufferDelay <= ceil(TimeDelta[i])` across RAPs, E.7.3
overflow, E.7.4 underflow when `LowDelayMode == 0`, E.7.5 minimum
decode time, E.7.6 minimum presentation interval, E.7.7 decode
deadline, E.7.8 `DecoderBufferDelay != 0` in schedule mode). The
Annex A.4 dynamic constraints (mirror
`annex-a-profiles-levels-and-tiers.md`, the `Removal[]`-dependent
clauses: `CompressedSize`, `FrameSymbolCount`,
`NumFrameHeadersPerSec` vs `MaxNumFrameHeadersPerSec`,
`TotalDisplayLumaSampleRate` vs `MaxDisplayRate`) consume the same
simulated times — they were explicitly deferred from the Annex A
skeleton (PR #46) for exactly this state.

This is validator arithmetic, not decoder work: every input is parsed
high-level syntax plus the segmenter's DFG boundaries; no pixel or
symbol decoding is involved.

## What Changes (as implemented — honest scope)

A careful decidability pass over Annex E concluded that most of the
simulation (E.5.1-.7, the E.6 resource-availability buffer-pool, and
the E.7.1-.7 conformance expressions) requires `CodedBits[i]` (DFG
byte accounting), `Removal[]` (incl. the E.5.5/E.6 buffer-pool replay
over `DecoderRefCount` / `PlayerRefCount` / `refresh_frame_flags`),
and/or `TimeToDecode[i]` (E.5.6: `MaxDecodeRate` + frame dims + the
`allow_global_intrabc && InloopFilteringEnabled` doubling), whose
inter-frame inputs route to **Unknown** on the current parse paths.
Firing those checks would risk false positives, so they are named
residuals (see the matrix `AV2-E-DECODER-MODEL` / `AV2-A-LEVELS-TIERS`
notes and `docs/VALIDATOR-DIAGNOSTICS.md`).

What landed is the fully-decidable, zero-false-positive subset:

1. **§ E.7.8** decoding-schedule-mode `DecoderBufferDelay` bound for the
   extended-layer arm — `!= 0` and `<= 90000 * (BufferSize / BitRate)`.
   The bound collapses to the exact constant **90000** for every
   defined level/tier (the `BitrateProfileFactor` and `MaxBitrate`
   cancel via the Table A.9 / Annex E.3 identities), so the check needs
   only the parsed schedule-mode flags + `decoder_buffer_delay` + the
   activated `seq_level_idx` / `seq_tier` — no per-DFG accounting.
   Decoding-schedule mode is gated on **all THREE** § E.4.2 conditions
   (mirror lines 293-296), not two: `decoder_model_info_present_flag == 1`,
   `seq_decoder_model_info_present_flag == 1`, AND
   `ci_timing_info_present_flag == 1` established for the extended layer
   (a content-interpretation record with `timing_info.is_some()` observed
   at/after the layer's § 7.3.8.11 random-access-point epoch — reusing the
   same "ci_timing established post-RAP" determination the § 6.16.7
   n_frames check applies). Absent the third condition the layer is not in
   schedule mode (E.4.2 closes that conformance cannot be checked, mirror
   lines 330-336), so the check suppresses (zero false positives). Two
   diagnostics: `decoder-model/schedule-decoder-buffer-delay-zero` and
   `decoder-model/schedule-decoder-buffer-delay-exceeds-bound`. The
   § E.7.1 `seq_level_idx == 31` exemption and the
   reserved/High-tier-below-4.0 honest-stops fall out of the
   defined-bitrate gate (`annex_a::bitrate_is_defined`). Named residual:
   the check evaluates once at first frame-confirmation (after the
   `emitted_annex_a_value_space` dedup), so an establishing CI that arrives
   only after that confirmation is a sound-over-complete miss.
2. The Table A.9 `MainMbps` / `HighMbps` definedness predicate it
   depends on, transcribed under `AV2-A-LEVELS-TIERS`.
3. The `brt/global-ordering-position` decision: **re-grounded** the
   recorded context TODO. The hard ordering check for a global BRT is
   still not implementable — it needs the § 7.3.8 per-RAP
   resource-availability removal schedule that this change did not
   model — so the global BRT stays unclassified (sound-over-complete)
   with explicit citations.

## Non-goals / named residuals

- The § E.5 frame-timing simulation, the E.6 buffer-pool, and the rest
  of § E.7 (E.7.1-.7, the E.7.8 OPS per-op arm) — blocked on
  `CodedBits[i]` / `Removal[]` / `TimeToDecode[i]` from Unknown-routing
  inter-frame state.
- The Annex A.4 dynamic rate constraints
  (`TotalDisplayLumaSampleRate`, `NumFrameHeadersPerSec`, per-frame
  `LumaSampleCount` / `CompressedSize` / `FrameSymbolCount` / tile
  bounds) — all consume `Removal[]` / `FrameParsingTime` / per-second
  output durations from the same unmodeled simulation;
  `FrameSymbolCount` additionally needs symbol decoding.
- Pixel/symbol decoding; display-model / HRD behavior.

## Acceptance criteria

- [x] The § E.7.8 schedule-mode bound fires on a provable violation
  with its citation; positive/negative/EOF/exempt/unconfirmed/
  no-decoder-model Unknown-routing cases per arm; the residuals are
  named with mirror citations; matrix rows updated with honest partial
  proof; `cargo xtask ci` green.
