# Design: Buffer-delay sum-constancy validation

## Context

Both syntax sites are already parsed in `splot-core`: `SequenceDecoderModelInfo`
(decoder/encoder buffer delays, `crates/splot-core/src/headers/sequence.rs`
~1739–1766, present only when the decoder-model flag is set) and the
per-operating-point `ops_decoder_buffer_delay` / `ops_encoder_buffer_delay`
(`crates/splot-core/src/headers/operating_point_set.rs` ~519–520, present only when
`ops_decoder_model_info_for_this_op_present_flag` is set). `ValidatorContext`
already owns the two state machines this change scopes against: exact § 7.3.6 CVS
boundaries (`CvsTracker`) and reset-aware per-`(xlayer, ops_id)`
`OperatingPointSetRecord` state (`crates/splot-validate/src/context.rs` ~120–121,
~529–589).

AVM research (2026-06-10) established that the reference implementation parses but
never enforces or consumes these values (the only consumer hardcodes its own
constants), so there is **no differential oracle** for these rules; proof is
hand-crafted vectors only.

## Goals / Non-Goals

**Goals:**

- Emit the error tier only where non-conformance holds under every plausible
  reading of § 6.4.13/§ 6.10.5 "video sequence"; emit the warning tier for
  broad-reading-only changes, with zero error-tier false positives by construction.
- Introduce the `decoder-model/` namespace with both registry gates co-evolved
  (`DIAGNOSTIC_PREFIXES` + `VALIDATOR-DIAGNOSTICS.md`).

**Non-Goals:**

- Annex E buffer simulation, BRT cross-checks, error-tier promotion of the warning
  cases, resolving the OPS-reset re-baselining ambiguity.

## Decisions

1. **Two ids, not one with two severities.** The registry maps one id to one
   severity; the conforming-under-some-readings cases get their own id
   (`decoder-model/buffer-delay-sum-changed-across-cvs`) so the error id's
   semantics stay provably sound.

2. **OPS tier (error) keys on `(obu_xlayer_id, ops_id, operating-point index)`.**
   Annex E binds `DecoderBufferDelay`/`EncoderBufferDelay` per `(xId, opsID, op)`
   (mirror `annex-e-decoder-model.md` lines 100–112). The check stores the last
   explicitly signaled sum per triple together with the CVS epoch and OPS-reset
   generation in which it was observed; a redefinition emits the error id only when
   CVS epoch and reset generation both match and the sum differs. Reuse the
   existing `OperatingPointSetRecord` Case-1–4 reset/update semantics — do not
   re-derive reset behavior.

3. **Seq-header tier (warning) compares activated headers only.** On each
   frame-confirmed sequence activation for an extended layer (the same
   `agreement_activation_for` decidability standard the § 6.4 checks use), if both
   the outgoing and incoming activated headers carry explicit
   `seq_decoder_model_info()` and the sums differ across a CLK boundary, emit the
   warning id. Fallback-guess activations never participate.

4. **Absent info is never compared.** A header or OPS entry without explicit
   decoder-model info contributes nothing and does not clear the stored sum;
   the Annex E mode defaults (70000/20000) are resource-availability fallbacks,
   not signaled values, and must not synthesize comparisons. Rationale comment with
   the `annex-e-decoder-model.md:261-272` citation at the comparison site.

5. **Suppression mirrors the sibling checks.** `ExternalHlsMode::Provided`
   suppresses both tiers (externally supplied HLS may legitimately differ), same as
   the landed `sequence-state/*` checks.

## Risks / Trade-offs

- [No oracle: a misreading survives testing] → quote § 6.4.13/§ 6.10.5 inline at
  each comparison; encode the three-readings soundness argument as a code comment
  on the error path; cover cross-CVS and reset-spanning cases as *negative* tests
  for the error id.
- [Warning-tier noise on legitimate splice points] → warning severity is the
  designed mitigation; the message text names the broad-reading assumption and the
  upstream clarification.
- [Registry gate failure on new namespace] → `DIAGNOSTIC_PREFIXES` and the
  registry tables must change in the same commit (`check-feature-status` and
  `check-diagnostic-registry` both gate CI).
- [Conflict with PR #38] → both PRs add registry rows in
  `VALIDATOR-DIAGNOSTICS.md`; whichever merges second rebases a 2-line table
  addition. No code overlap (this change touches OPS/decoder-model paths, #38
  touches CMVS/monotonic paths).

## Migration Plan

Pure addition of diagnostics and crate-private state; no public API changes.
Rollback is reverting the PR.

## Open Questions

- Upstream: the exact scope of "video sequence" in § 6.4.13/§ 6.10.5 (AOMedia
  clarification request being drafted separately; resolves whether the warning
  tier can ever be promoted).
- Whether an OPS reset re-baselines the constraint for a reused `ops_id`
  (documented in-code; warning tier covers the spanning case).
