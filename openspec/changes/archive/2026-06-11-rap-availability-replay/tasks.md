# Tasks: RAP availability replay

## 1. Bookkeeping

- [x] 1.1 Matrix `openspec_change` on the three rows; register in
  `openspec/changes/README.md`; re-read § 7.3.8.1/§ 7.4.1 mirror text and
  the per-family § 7.3.8.x availability subsections actually modeled.

## 2. Replay core

- [x] 2.1 Resend-TU epochs on the HLS/OPS stores (every family they track);
  RAP-TU detection reuses the § 7.4.1 groundwork.
- [x] 2.2 The replay diagnostic(s) per registry conventions, § 7.3.8.1
  citation, family in the message, dedup per (object, RAP), severity and
  ExternalHlsMode handling per the existing `hls/unavailable-*` pattern and
  the documented partial-declaration policy.
- [x] 2.3 Leading-TU soundness: LEADING_*-bearing TUs disqualify a resend;
  undecidable TUs qualify (documented under-approximation).

## 3. § 7.3.8.2 MSDO at-RAP availability

- [x] 3.1 The deferred first-sentence check joins the replay machinery
  (resolve the matrix note's deferral from PR #47). Resolved by honest
  re-scope: §7.3.8.2's second sentence ("requirements on the presence of MSDO
  OBUs depend on the interoperability point, as specified in Annex A.2") +
  §7.4.1 step 2 (a RAP may legitimately drop MSDO, setting
  MultiStreamDecoderMode==0 / ending the CMVS) make a missing-MSDO-at-RAP
  interop-point-dependent, so firing a generic error would false-positive on a
  conformant multistream→single-stream RAP transition. Kept as a named residual
  blocked on the Annex A.2 MSDO-presence model (home: msdo-global-lcr-agreement
  / Table A.4 IOP machinery); MSDO has no §7.3.8.1-style reference site to
  buffer (it is a presence requirement, not a referenced object), so the
  §7.3.8.1 replay machinery does not apply to it. Matrix notes on AV2-5.6-MSDO
  / AV2-7.3.8-HLS-AVAILABILITY record this. (See final report.)

## 4. Cross-xlayer TODO

- [x] 4.1 Resolve the context.rs ~5407 cross-xlayer seq_header_id
  validation TODO per its note (verify what it defers and whether this
  change's state enables it; if not, re-scope the TODO honestly). Resolved by
  re-scope: §6.4.1 models the header memory globally (mirror line 641), so the
  replay key is already global; the remaining gap is the §7.3.6 per-layer
  bit-identity fingerprint — the TODO is rewritten with that basis and
  retargeted to AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT.

## 5. Docs, registry, artifacts

- [x] 5.1 Register ids; matrix rows advance with proof
  (AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS residual resolves;
  AV2-5.7-MULTI-FRAME-HEADER availability half completes with its frame-size
  bounds residual named; the ExternalHlsSet extension and atlas-global residuals
  named on AV2-7.3.8-HLS-AVAILABILITY); regenerate generated docs; roadmap
  Phase 4/5 mentions updated.

## 6. Verification

- [x] 6.1 Tests per acceptance criteria (per family; both RAP kinds
  CLK/OLK/RAS where applicable; multi-RAP dedup; external-mode handling).
- [x] 6.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 6.3 `cargo xtask ci` (bare, exit checked) passes (exit 0).
