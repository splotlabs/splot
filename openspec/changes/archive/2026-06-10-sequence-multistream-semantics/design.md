# Design: Sequence-header multistream semantics

## Context

`ValidatorContext` (`crates/splot-validate/src/context.rs`) already models the two
prerequisites these checks were waiting on:

- exact per-xlayer §7.3.6 CVS boundaries via `CvsTracker`
  (`observe_cvs_boundary_events` → `start_cvs_for_xlayer`), with deferred cross-TU
  diagnostics;
- per-frame §5.18.2 sequence activation with frame-confirmed tracking
  (`observe_frame_bearing_obu`, `active_sequence_by_xlayer`,
  `frame_confirmed_xlayers`), distinguishing confirmed activations from the OBU-order
  fallback guess.

`splot-core` exposes every parsed input needed: `SequenceHeaderGeneral::
seq_max_mlayer_count`, `monotonic_output_order_flag` (single-picture inference applied
at parse), `mlayer_dependency_map` (indexed `depends_on` access), the per-OBU
`(obu_xlayer_id, obu_mlayer_id, obu_tlayer_id)` triple, and the full §5.6
`MultistreamDecoderOperation` model including the six §7.3.2 condition-2 fields
(`crates/splot-core/src/hls.rs`). No new parsing is required.

## Goals / Non-Goals

**Goals:**

- Emit the four diagnostics from the proposal with zero false positives on conforming
  streams (sound under-approximation wherever scope is statefully uncertain).
- Build the minimal §7.3.2 CMVS begin/end tracker and stateful MSDO observer that the
  monotonic check needs and the future §7.3.2–§7.3.5 milestone will extend.
- Correct the stale matrix note and roadmap mis-citation in the same change.

**Non-Goals:**

- `cmvs/*` diagnostics, frame-unit segmentation, LTR availability, OrderHint-based
  checks, §6.4.13 buffer-delay constancy (maintainer question), anything behind the
  roadmap fence.

## Decisions

1. **Check placement follows existing seams.**
   - `frame-header/switch-or-ras-mlayer-dependency-not-self-contained` lands in
     `frame_header_core_checks` (context.rs ~:4339), which already receives the OBU
     envelope plus the active `SequenceHeader` and implements the analogous §6.4.6 RAS
     check at its top. No new state.
   - `sequence-state/distinct-mlayer-count-exceeds-seq-max` collects
     `obu_mlayer_id` values in `observe_obu`, keyed per extended layer, cleared at each
     CVS start from `CvsTracker`; compared against the active header's
     `seq_max_mlayer_count` (emit once per CVS on first exceedance).
   - `hls/multiple-active-sequence-headers` hooks the §5.18.2
     `load_sequence_header` activation path (context.rs ~:1508–1522), which already
     computes `previous != Some(seq_id)`.
   - `sequence-state/monotonic-output-order-mismatch` fires on sequence activation
     while the CMVS tracker reports definitively-inside-a-CMVS, comparing the activated
     header's flag against the other xlayers' active headers.

2. **CMVS tracker is a sound under-approximation.** §7.3.2 (mirror
   `07-decoding-process.md` lines 323–344) defines begin conditions (TU containing a
   CLK plus: no-CMVS-active + MSDO present; CMVS-active + MSDO with a changed key
   field; no-CMVS-active + global LCR activated) and end conditions (new CMVS; CVS
   start that is neither MSDO-accompanied nor global-LCR-activated; end of bitstream).
   The tracker exposes three states: `Outside`, `Inside`, `Unknown`. Checks gated on
   the CMVS only fire in `Inside`. Every transition carries an inline spec quote
   (mitigates misreading §7.3.2 — there are no real multistream conformance vectors
   yet, so the spec text is the only oracle).

3. **MSDO observer.** The validator currently touches `Msdo` exactly once
   (context.rs ~:4200). A stateful observer records the last-seen MSDO per scope and
   detects §7.3.2 condition-2 key-field changes. It feeds only the CMVS tracker in
   this change.

4. **Distinct-mlayer attribution is resolved conservatively.** §6.4.1 counts
   `obu_mlayer_id` over "the coded video sequence associated with this sequence
   header" and the NOTE says all OBUs count, even non-layer-specific ones. Attribution
   of global-xlayer OBUs to a per-xlayer CVS must be settled against the mirror's
   §6.2.2/§7.3.6 wording during implementation; whichever reading is taken, the check
   only counts OBUs whose attribution is unambiguous under that reading (documented in
   a code comment with the citation), so an exceedance report is always real. Any
   genuinely ambiguous residue becomes a `TODO(spec:
   AV2-6.4-SEQUENCE-HEADER-SEMANTICS)`. **CLK boundary re-attribution (follow-up):**
   §7.3.6 (mirror `07-decoding-process.md` lines 604-606) starts the new CVS AT the
   temporal unit containing the CLK, so `DistinctMlayerTracker::reset_cvs` re-attributes
   the boundary temporal unit's pre-CLK ids (canonically the §7.3.8.1 resent-at-RAP
   sequence header at `obu_mlayer_id == 0`) to the new CVS rather than dropping them; a
   per-temporal-unit seen set drives the re-seed, ids from earlier temporal units never
   enter, and the deferred old-CVS exceedance still drops at the boundary. This replaces
   the original documented sound under-count with exact re-attribution.

5. **`hls/multiple-active-sequence-headers` gating.** Emit only when (a) the previous
   activation for the xlayer is frame-confirmed (`frame_confirmed_xlayers`), (b) no
   CVS start intervened (`CvsTracker`), (c) the newly activated `seq_header_id`
   differs, and (d) caller-provided external HLS declares at least one sequence header
   (`ExternalHlsSet::declares_any_sequence_header()`). Gate (d) is narrowed from the
   broad `Provided(_)` to the same predicate the sibling gates and
   `validate_active_sequence_limits` use: only a *declared* external sequence header can
   be the out-of-band active header that makes the in-band activation history
   unreliable; an external channel declaring no sequence header (an empty or
   sequence-free `ExternalHlsSet`) cannot, so it does not suppress. The same narrowing
   applies to the `sequence-state/monotonic-output-order-mismatch` gate. Severity
   `error` (the roadmap's "warning until CLK parsing exists" hedge is obsolete); the
   escalation is called out in the proposal for reviewer sign-off.

## Risks / Trade-offs

- [CMVS begin/end misinterpretation] → three-state tracker with `Unknown`, checks fire
  only on `Inside`, inline spec quotes at each transition, dedicated begin/end
  transition tests. The monotonic check additionally defers a sequence-header-time
  `Inside` verdict that no CLK has yet confirmed (provisional `Inside`), resolving it at
  temporal-unit completion, so a §7.3.6-permitted same-CVS redefinition immediately
  preceding a CMVS-ending CLK is not a false positive.
- [distinct-mlayer false positives at CVS boundaries / global-OBU attribution] →
  conservative global-OBU attribution (decision 4) plus exact CLK-boundary
  re-attribution (decision 4 follow-up) restricted to the boundary temporal unit's ids,
  with boundary edge tests (pre-CLK header re-attributed and flagged, earlier-temporal-
  unit ids excluded, once-per-CVS across the boundary, deferred old-CVS exceedance
  dropped).
- [multiple-active false positives from fallback activation guesses] → gate on
  `frame_confirmed_xlayers` and on caller-provided external HLS that declares a sequence
  header (`declares_any_sequence_header()`, decision 5).
- [Synthetic-only fixtures] → all multistream tests are `Bits`-built Annex B streams
  (MSDO + per-xlayer CLKs); `avm_diff` stays `pending` in the matrix — no stage is
  marked beyond what synthetic proof supports.
- [Registry/ledger drift] → the four ids are hand-added to
  `docs/VALIDATOR-DIAGNOSTICS.md` (CI `check-diagnostic-registry` enforces exact set
  equality); generated docs regenerated and audit ledger re-recorded in the same PR.

## Migration Plan

Pure addition of diagnostics and crate-private state; no public API or CLI changes.
Rollback is reverting the PR.

## Open Questions

- §6.4.13 buffer-delay sum constancy scope ("video sequence" vs "coded video
  sequence") — maintainer question, excluded from this change.
- Severity sign-off for `hls/multiple-active-sequence-headers` (error vs the roadmap's
  old warning hedge) — flagged for the PR reviewers.
