# validator delta: obu-header-rap-activation-window

Closes the § 6.2.2 NOTE residual on `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` by scoping
the `obu_tlayer_id` / `obu_mlayer_id` limit to the activated header's window rather than the
parse-order live store.

## ADDED Requirements

### Requirement: § 6.2.2 layer-id limits follow the activated header window

The validator SHALL evaluate the § 6.2.2 `obu_tlayer_id <= max_tlayer_id` and
`obu_mlayer_id <= max_mlayer_id` requirements
(docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-2, lines 190-194) against the
limits of the sequence header *as activated* by the latest § 5.18.2 frame-confirmed
`load_sequence_header`, honoring the § 6.2.2 NOTE (lines 197-198) that the constraints apply
only after a sequence header is activated. A § 7.3.6 same-`seq_header_id` redefinition (legal
only at a coded-video-sequence boundary) that has been sent but not yet re-activated by its
confirming frame SHALL NOT re-scope the limit for an OBU still in the prior activation's window.
For a layer whose active sequence header is only the first-seen OBU-order fallback (no frame
confirmation), the validator SHALL continue to evaluate the limit against the live store. This
introduces no new `rule_id`; it refines when `sequence-state/tlayer-exceeds-max` and
`sequence-state/mlayer-exceeds-max` (§ 6.2.2) fire.

#### Scenario: tightening redefinition before its re-confirming frame

- **WHEN** a frame-confirmed sequence header is redefined (same `seq_header_id`) with a tighter
  `max_tlayer_id` at a coded-video-sequence boundary, and a non-global OBU whose `obu_tlayer_id`
  conforms to the prior activated `max_tlayer_id` (but not the redefined one) appears before the
  CLK frame that re-activates the redefinition
- **THEN** no `sequence-state/tlayer-exceeds-max` diagnostic is produced for that OBU

#### Scenario: violation of the prior activated limit still fires

- **WHEN** an OBU in that same pre-re-confirmation window has an `obu_tlayer_id` exceeding even
  the prior activated `max_tlayer_id`
- **THEN** an error diagnostic `sequence-state/tlayer-exceeds-max` (§ 6.2.2) is produced

#### Scenario: the snapshot advances on re-activation

- **WHEN** the CLK frame re-activates the tightened redefinition, and a later OBU has an
  `obu_tlayer_id` exceeding the now-activated `max_tlayer_id`
- **THEN** an error diagnostic `sequence-state/tlayer-exceeds-max` (§ 6.2.2) is produced

#### Scenario: fallback-only layers are unchanged

- **WHEN** a layer's active sequence header is only the first-seen OBU-order fallback, with no
  frame-confirmed activation
- **THEN** the § 6.2.2 limit is evaluated against the live store exactly as before

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
