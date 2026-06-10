# Proposal: Buffer-delay sum-constancy validation

## Why

AV2 v1.0.0 § 6.4.13 requires that "For a video sequence that includes one or more
random access points the sum of decoder_buffer_delay and encoder_buffer_delay shall
be kept constant" (`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md` lines
1301–1302), with the per-operating-point analogue in § 6.10.5 (mirror lines
2810–2825). The sentence is ambiguous — "video sequence" is not a defined term and
the "requirement of bitstream conformance" formula is absent — so this was held as a
maintainer question in the `sequence-multistream-semantics` proposal. The question
has now been resolved by direct AVM research plus a maintainer decision (2026-06-10):

- **AVM never enforces the constraint.** It parses both syntax sites
  (`avm/av2/decoder/obu.c:625-643`, `avm/av2/decoder/obu_ops.c:57-66`) and the only
  consumer of the sum anywhere is the encoder-side level-conformance smoothing-buffer
  model, which hardcodes 70000/20000 and never reads signaled values
  (`avm/av2/common/level.c:803-806`, `:961-963`). At random access points the new
  sequence header is activated by wholesale copy with no comparison, and OPS
  redefinition overwrites the slot with no value check — AVM is silent at exactly the
  points where the sentence is non-vacuous.
- **The sentence is AV1 boilerplate that lost its intra-CVS force in AV2.** AV1
  exempted per-operating-point delays from sequence-header bit-identity, making the
  sentence meaningful within a CVS; AV2 moved per-OP delays into OPS OBUs and has no
  such exemption (§ 7.3.6 bit-identity, mirror `07-decoding-process.md` lines
  604–610), so under a per-CVS reading the seq-header variant is vacuous. Any
  non-vacuous reading must span CVS boundaries.
- **Maintainer decision:** implement a two-tier check (error + warning severity
  split, approved 2026-06-10) rather than defer; the validator catching what the
  oracle ignores is splot's core value proposition, and hand-crafted or hostile
  streams can exercise this syntax even though AVM's encoder never emits it (its
  writer paths are statically dead: `avm/av2/encoder/encoder.c:1007-1012`,
  `avm/av2/encoder/bitstream_ops.c:259-280`).

## What Changes

Two new diagnostics in a new `decoder-model/` rule-id namespace:

- `decoder-model/buffer-delay-sum-changed` — severity **error**. Fires only for the
  sub-case that is non-conforming under *every* plausible reading of "video
  sequence" (CVS, CMVS, or whole per-xlayer sub-bitstream): the same
  `(obu_xlayer_id, ops_id, operating-point index)` is redefined **within one coded
  video sequence** with **no intervening OPS reset**, **both** the old and new
  signaling explicitly carry decoder-model info, and the
  `ops_decoder_buffer_delay + ops_encoder_buffer_delay` sum differs. The
  "includes one or more random access points" qualifier always holds because every
  CVS starts at a closed random access point.
- `decoder-model/buffer-delay-sum-changed-across-cvs` — severity **warning**
  (advisory). Fires for the broad-reading-only cases: the activated sequence
  header's `seq_decoder_model_info()` sum changing across a CLK boundary within the
  same extended layer, or an OPS sum changing across a CVS or OPS-reset boundary
  for the same operating-point triple. These are conforming under the per-CVS
  reading, so error severity would violate the zero-false-positive rule. This tier
  asserts the broad reading and must never be promoted to error without an upstream
  AOMedia clarification.

Comparison rules (both tiers):

- Only **explicitly signaled** values participate. Absent decoder-model info —
  including the Annex E resource-availability defaults
  (`DecoderBufferDelay = 70000` / `EncoderBufferDelay = 20000`, mirror
  `annex-e-decoder-model.md` lines 261–272) — never enters a comparison.
- State is keyed per extended layer (seq-header tier) and per
  `(obu_xlayer_id, ops_id, operating-point index)` (OPS tier), reusing the existing
  `CvsTracker` CVS boundaries and the reset-aware OPS record state.
- No intra-CVS seq-header check is added: § 7.3.6 bit-identity
  (`hls/repeated-sequence-header-not-identical`) already subsumes it, mirroring
  AVM's `memcmp`.

Namespace co-evolution (required by the registry gates):

- Add `decoder-model/` to `DIAGNOSTIC_PREFIXES` in `xtask/src/feature_status.rs`
  and document its rules in `docs/FEATURE-TRACKING.md` § 12.
- Register both ids in `docs/VALIDATOR-DIAGNOSTICS.md`, with an explicit note that
  **no AVM differential oracle exists** for these rules (AVM is parse-only here);
  proof is hand-crafted unit/snapshot vectors only.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `validator`: two ADDED requirements — intra-CVS OPS buffer-delay sum constancy
  (error) and cross-boundary buffer-delay sum advisory (warning).

## Impact

- `crates/splot-validate/src/context.rs` (new per-xlayer / per-OPS-triple sum
  state; checks hooked into sequence activation and OPS observation paths) and
  `crates/splot-validate/src/validator.rs` (tests).
- `xtask/src/feature_status.rs` (`DIAGNOSTIC_PREFIXES` gains `decoder-model/`).
- No `splot-core` parsing changes: `SequenceDecoderModelInfo`
  (`crates/splot-core/src/headers/sequence.rs` ~1739–1766) and the OPS delay fields
  (`crates/splot-core/src/headers/operating_point_set.rs` ~519–520) are already
  parsed.
- Matrix rows: the § 6.4.13 semantics tier tracks under
  `AV2-6.4-SEQUENCE-HEADER-SEMANTICS`; the § 6.10.5 tier under
  `AV2-5.11.3-OPS-DECODER-MODEL-INFO` (find-or-create per
  `docs/FEATURE-TRACKING.md`; record the AVM-non-enforcement finding in the row
  notes).
- Docs: `VALIDATOR-DIAGNOSTICS.md`, `FEATURE-TRACKING.md` § 12,
  `VALIDATOR-ROADMAP.md` (the § 6.4.13 maintainer-question entry resolves),
  generated `FEATURE-STATUS.md`/`SPEC-COVERAGE.md`, audit ledger.

## Non-goals

- Promoting the warning tier to error (blocked on upstream AOMedia clarification of
  "video sequence" scope — an upstream request is being drafted separately).
- Decoder-model buffer *simulation* (Annex E smoothing-buffer arithmetic,
  `AV2-E-DECODER-MODEL`): this change compares signaled values only.
- Whether an OPS reset re-baselines the constraint for a reused `ops_id` is itself
  ambiguous; reset-spanning comparisons stay in the warning tier (sound choice, may
  under-report) with a code comment recording the ambiguity.
- Buffer-removal-timing (§ 5.12 BRT) semantics and any cross-check against BRT
  values.

## Feature IDs

- `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` (§ 6.4.13 seq-header tier)
- `AV2-5.11.3-OPS-DECODER-MODEL-INFO` (§ 6.10.5 OPS tier)
- `AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT` (CVS scoping prerequisite, already landed)

## Acceptance criteria

- Both diagnostics ship with spec citations (§ 6.4.13 / § 6.10.5 + mirror paths),
  positive/negative/boundary tests (CVS boundary, OPS reset, absent-info
  non-comparison, Annex E defaults never compared), and registry entries; matrix
  stages advance only with proof.
- The error tier provably cannot fire on a stream that is conforming under *any* of
  the three candidate readings (test the cross-CVS and reset-spanning cases as
  negative tests for the error id).
- `cargo xtask ci`, `check-feature-status`, and `check-diagnostic-registry` pass
  with the new namespace.
