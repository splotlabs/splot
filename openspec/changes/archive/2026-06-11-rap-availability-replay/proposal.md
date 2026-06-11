# Proposal: random-access-point HLS availability replay (§ 7.3.8.1)

## Feature IDs

- `AV2-7.3.8-HLS-AVAILABILITY` (the replay core)
- `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` (its named residual)
- `AV2-5.7-MULTI-FRAME-HEADER` (availability half)

## Why

§ 7.3.8.1 (mirror lines 679–700) requires every referenced HLS OBU to be
available "if decoding process starts at any random access point and drops
any temporal units containing leading frames" — the NOTE spells it out: HLS
used at a RAP must be resent in the RAP's temporal unit or provided
externally. The validator checks linear-decode availability today but never
replays availability from RAPs, so a stream whose sequence header / MFH /
film-grain model / MSDO / OPS / LCR was only sent before the RAP passes
validation while failing real random access. RAP temporal units are
OBU-type-detectable (§ 7.4.1, the PR #47 groundwork), making the sound
subset implementable now.

## What Changes

1. **Replay epochs**: the HLS/OPS availability stores record each object's
   most recent (re)send temporal unit. At a § 7.4.1 RAP temporal unit, a
   reference at/after the RAP to an HLS object whose most recent qualifying
   resend precedes the RAP fires a diagnostic citing § 7.3.8.1 (one rule id
   per the registry's namespace conventions; the OBU family named in the
   message). Severity follows the existing `hls/unavailable-*` pattern's
   handling of `ExternalHlsMode` (the external-means escape: under
   `Provided`, externally-declarable kinds are suppressed per the documented
   partial-declaration policy; under `Disabled`, the caller asserts no
   external provision).
2. **Leading-TU soundness**: a resend inside a temporal unit containing
   LEADING_* OBU types does not satisfy the replay (those TUs drop under
   random access); temporal units whose leading-ness is undecidable count as
   satisfying (under-report, never false-positive). Documented.
3. **§ 7.3.8.2 availability-at-RAP** for MSDO (the sentence deferred from
   PR #47) joins the same replay machinery.
4. **Cross-xlayer seq_header_id validation TODO** (context.rs ~5407)
   resolved per its own note.
5. Matrix: `AV2-7.3.8-HLS-AVAILABILITY` advances;
   `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS`'s "until § 7.3.8 availability
   lands" residual resolves; `AV2-5.7-MULTI-FRAME-HEADER`'s availability half
   completes (its § 6.17.2 frame-size bounds stay blocked on reference state,
   named).

## Non-goals

- Extending `ExternalHlsSet` with MFH/LCR/atlas declaration keys: that
  reopens the settled partial-declaration suppression policy (PR #49) and
  belongs to its own change if ever needed; the policy already covers the
  escape conservatively. Recorded as a named residual.
- Frame-header-derived leading-frame classification for non-LEADING_* OBU
  types (blocked on inter frame parsing; the type-detectable subset is the
  sound under-approximation).
- The global atlas reference modeling deferred at the
  AV2-7.3.8-HLS-AVAILABILITY row (stays named).

## Acceptance criteria

- [ ] Replay violations fire per family (sequence header, MFH, film grain,
  MSDO, OPS, LCR, atlas — whatever the stores track) with resend-in-RAP-TU
  passing, pre-RAP-only failing, leading-TU resend failing (when
  detectable), undecidable TUs passing, and ExternalHlsMode handling per
  the established policy; dedup so one dangling object reports once per
  RAP.
- [ ] Matrix rows advance with proof; registry/feature-status/ci/coverage
  gates pass.
