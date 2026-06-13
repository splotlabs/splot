# Tasks: Annex E decoder-model schedule

## 1. Bookkeeping

- [x] 1.1 Matrix rows confirmed; `openspec_change` set on every
  touched row (`AV2-E-DECODER-MODEL`, `AV2-A-LEVELS-TIERS`,
  `AV2-7.3-OBU-ORDERING`); read Annex E.3-E.7 verbatim (mirror
  `annex-e-decoder-model.md`), § 6.11 `br_time` semantics, the
  Annex A.4 dynamic clauses, and the parsed
  decoder-model/buffer-removal/temporal-point syntax rows.

## 2. Implementation

- [~] 2.1 Annex E timing state per selected operating point: DFG
  arrival/removal/decode/presentation times (E.5.1-.7), schedule vs
  resource-availability modes (E.4). **Named residual** — blocked on
  `CodedBits[i]` / `Removal[]` / `TimeToDecode[i]` from Unknown-routing
  inter-frame state (see proposal Non-goals + matrix
  `AV2-E-DECODER-MODEL`). Only the schedule-mode determination for the
  extended-layer arm (the THREE E.4.2 conditions:
  `decoder_model_info_present_flag`, `seq_decoder_model_info_present_flag`,
  and the layer's established `ci_timing_info_present_flag == 1`) is
  consumed, for E.7.8.
- [x] 2.2 § E.7 conformance — landed the decidable subset: **§ E.7.8**
  schedule-mode `DecoderBufferDelay` bounds (extended-layer arm), gated on
  all THREE § E.4.2 schedule-mode conditions (the third being
  `ci_timing_info_present_flag == 1` established for the layer at/after its
  § 7.3.8.11 RAP epoch, reusing the § 6.16.7 n_frames "ci_timing
  established" determination), with the `seq_level_idx == 31` exemption.
  Named residual: evaluation is once at first frame-confirmation (after the
  `emitted_annex_a_value_space` dedup), so a CI establishing ci_timing only
  after that confirmation is a sound-over-complete miss. The other § E.7
  expressions are named residuals (need the E.5/E.6 simulation).
- [~] 2.3 Annex A.4 dynamic rate constraints — **named residual**: all
  consume `Removal[]` / `FrameParsingTime` / per-second output
  durations from the unmodeled resource-availability simulation. The
  Table A.9 `MainMbps` / `HighMbps` definedness predicate (backing the
  E.7.8 bound) landed.
- [x] 2.4 The `brt/global-ordering-position` decision — **re-grounded**:
  the hard check still needs the § 7.3.8 per-RAP removal schedule this
  change did not model; the TODO + matrix `AV2-7.3-OBU-ORDERING` note
  now carry explicit citations for why it stays unclassified.
- [x] 2.5 Unknown routing: undefined-bitrate honest-stop, the
  `seq_level_idx == 31` exemption, non-schedule-mode silence (incl. the
  third-condition silence — no CI, CI with `ci_timing_info_present_flag ==
  0`, and a pre-RAP CI reset to 0 by an OLK — each with an anti-vacuity
  firing pair), unconfirmed-activation silence, and truncated-header
  silence — all tested.

## 3. Docs

- [x] 3.1 Registry entries (two new `decoder-model/` ids with § E.7.8
  citations + updated namespace footer); matrix proof; named residuals;
  roadmap updates.

## 4. Verification

- [x] 4.1 Positive/negative/EOF + exempt/unconfirmed/no-decoder-model
  Unknown-routing tests; the bound-derivation table tests.
- [x] 4.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 4.3 `cargo xtask ci` (bare, exit checked) passes.
