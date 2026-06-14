# Change: obu-header-rap-activation-window

## Feature IDs

- `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS`

## Why

Closes the last residual on `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS`
(`validate = partial`). The `obu_tlayer_id <= max_tlayer_id` /
`obu_mlayer_id <= max_mlayer_id` checks (§ 6.2.2, mirror
`06-syntax-structures-semantics.md` lines 190-194) already landed
(`sequence-state/tlayer-exceeds-max`, `sequence-state/mlayer-exceeds-max`). The residual is
the § 6.2.2 NOTE (mirror lines 197-198): "These constraints on obu_tlayer_id and
obu_mlayer_id apply **after** a sequence header OBU is activated to specify max_tlayer_id and
max_mlayer_id." The limits are scoped to the *activated* header's window, but the check read
the active id's limits from the live store instead of the header as activated.

The two diverge in exactly one window. § 7.3.6 permits a same-`seq_header_id` redefinition
only at a coded-video-sequence boundary (a CLK), and § 5.18.2 `load_sequence_header`
activation is frame-driven. A redefinition overwrites the live store the moment it is sent,
but does not re-activate until its confirming frame. An OBU between the redefinition and that
frame — "between a random access point and its resend-confirmed sequence-header activation" —
is still in the *prior* activation's window, yet the check compared it against the
redefined (e.g. tightened) limits. When the redefinition tightens the limit, an OBU that
conforms to the in-force activated limit is then a false positive; when it loosens the limit,
a genuine prior-window violation is under-reported.

This is the same shape the § 6.4.1 distinct-`obu_mlayer_id` check already handles by comparing
against the CLK-*activated* header rather than the outgoing/stored one
(`distinct_mlayer_count_reattribution_compares_against_clk_activated_header`). The § 5.18.2
frame-confirmed activation path (the validator's activation model, underpinning the § 7.3.8.1
RAP-availability replay) already exists; this change consumes it for the § 6.2.2 limit.

## What changes

The § 6.2.2 limit check evaluates `obu_tlayer_id` / `obu_mlayer_id` against the limits of the
header *as activated* by the latest § 5.18.2 frame-confirmed `load_sequence_header`, snapshotted
at activation, rather than re-reading the active id's payload from the live store. A
frame-confirmed layer's snapshot advances only when a confirming frame re-activates the header,
so a redefinition that is stored but not yet re-activated does not retroactively re-scope the
limit for OBUs still in the prior activation's window. A layer with only the first-seen
OBU-order fallback (no frame confirmation) keeps reading the live store, preserving the eager
pre-frame behavior.

## Scope

- Spec sections: § 6.2.2 (OBU header layer-id limits + the activation-window NOTE), § 5.18.2
  (`load_sequence_header` frame-confirmed activation), § 7.3.6 (CVS-boundary redefinition).
- Crates/modules: `crates/splot-validate/src/context/mod.rs` (new
  `frame_confirmed_activated_limits` snapshot field),
  `crates/splot-validate/src/context/frame_headers.rs` (snapshot the activated limits on the
  frame-confirmed activation path), `crates/splot-validate/src/context/sequence.rs`
  (`validate_active_sequence_limits` reads the snapshot for frame-confirmed layers).
- Docs/tests: matrix notes (`AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS`,
  `validate = partial -> done`), three `sequence_state.rs` tests. No new `rule_id` (refines when
  the existing `sequence-state/tlayer-exceeds-max` / `mlayer-exceeds-max` fire).

## Non-goals

- No new diagnostics and no change to the OBU-order fallback path (fallback-only layers keep
  reading the live store).
- The orthogonal § 7.3.8.1 HLS-*availability* replay (`hls/unavailable-at-random-access-point`)
  is unchanged; this change refines the § 6.2.2 *limit* check, which depends on the activation
  window, not on object availability.

## Acceptance criteria

- [ ] `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` notes updated; `validate` flips
      `partial -> done`.
- [ ] Negative (no false positive): an OBU between a tightening § 7.3.6 redefinition and its
      re-confirming CLK frame, conforming to the prior activated `max_tlayer_id`, does not fire
      `sequence-state/tlayer-exceeds-max`.
- [ ] Positive (no over-suppression): an OBU in the same window exceeding even the prior
      activated limit still fires.
- [ ] Positive (snapshot advances): after the CLK re-confirms the tightened header, a later OBU
      exceeding the new limit fires.
- [ ] Fallback-only layers and all existing `active_sequence_header_*` behavior unchanged.
- [ ] `cargo xtask ci` passes.
