# Proposal: MSDO sub-stream constraint checks (§ 6.6, § 7.3.8.2)

## Feature IDs

- `AV2-5.6-MSDO`
- `AV2-7.3.8-HLS-AVAILABILITY` (the § 7.3.8.2 identity rule)

## Why

The MSDO row's remaining § 6.6 "requirement of bitstream conformance"
sentences and the § 7.3.8.2 non-RAP identity rule are all decidable from
already-parsed state (the audit's matrix-note claim that § 7.3.8.2 needs
frame-header activation was wrong: § 7.4.1 defines a random access point
purely by OBU-type presence in the temporal unit — CLK/OLK/RAS — which raw
headers already give us). These checks also build exactly the MSDO state the
descoped Table A.4 IOP-presence machinery (PR #46) needs to re-land next.

## What Changes

New `msdo/` diagnostics, grounded in
`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-6` (mirror lines
1330–1398) and `07-decoding-process.md#s-7-3-8-2` (line 716) / `#s-7-4-1`
(lines 1007–1019):

1. `msdo/profile-below-substream-max` (error, § 6.6 line 1347):
   `multistream_profile_idc < sub_stream_max_profile[i]` for any i.
2. `msdo/substream-profile-exceeds-max`, `msdo/substream-level-exceeds-max`,
   `msdo/substream-tier-exceeds-max` (errors, § 6.6 lines 1365–1378): a
   sequence header activated by the i-th sub-stream (mapped via
   `sub_xlayer_id[i]`) with `seq_profile_idc` / `seq_level_idx` / `seq_tier`
   above the declared `sub_stream_max_*[i]`. Gated on **frame-confirmed**
   activation (the PR #46 decidability rule); evaluated both when a header
   activates under a recorded MSDO and when an MSDO arrives with headers
   already active.
3. `msdo/doh-constraint-required` (error, § 6.6 line 1391):
   `monotonic_output_order_flag == 0` in any activated sequence header of the
   coded multistream video sequence while `multistream_doh_constraint_flag ==
   0`; gated on `CmvsState::Inside` exactly like the landed
   monotonic-output-order agreement check.
4. `msdo/non-rap-not-identical` (error, § 7.3.8.2): an OBU_MSDO in a temporal
   unit that is not a random access point differs from the previous OBU_MSDO.
   Because § 7.3.7 ordering puts global HLS before the frame OBUs, the TU's
   RAP-ness (§ 7.4.1: contains CLK/OLK/RAS) is only known at TU end — the
   MSDO payload fingerprint is buffered per TU and compared at TU end.
5. `annex-a/profile-reserved` also fires for a reserved
   `multistream_profile_idc` (§ 6.6 says its value space is
   `seq_profile_idc`'s; the mirror's "Table A.4" reference is a spec
   erratum for Table A.1 — note it, do not invent semantics beyond the value
   space).
6. Roadmap hygiene: the planned `msdo/sub-xlayer-duplicate` backlog row is
   struck (re-verify first): § 6.6 mirror lines 1330–1398 contain no
   `sub_xlayer_id` uniqueness requirement; spec honesty forbids inventing
   one.

Checks 1 and 5 are locally decidable at the MSDO OBU. The matrix `AV2-5.6-MSDO`
row advances with proof; its § 7.3.8.2 note correction (OBU-type-detectable,
not frame-blocked) is part of this change.

## Non-goals

- Table A.4 IOP presence re-land (next change, msdo-global-lcr-agreement —
  but the MSDO state recorded here must be reusable for it).
- §7.3.8.2 first-sentence availability ("shall be available at each random
  access point... or by provision through external means") — availability
  modeling belongs to `rap-availability-replay` (backlog item 10).
- MultiStreamDecoderMode==1 substream level scaling (deferred on
  `AV2-A-LEVELS-TIERS`).
- The § 7.3.7 DOH constraints the flag *enables* (item `celu-orderhint-
  constraints`); this change checks only the flag-requirement sentence.

## Acceptance criteria

- [ ] Every diagnostic cites § 6.6 or § 7.3.8.2 with mirror paths; positive,
  negative, and boundary tests per check (equality passes the max checks;
  +1 fails; RAP TU exempts the identity check; identical MSDO passes it).
- [ ] The substream-max checks fire in both arrival orders (MSDO-then-header,
  header-then-MSDO) and only for frame-confirmed activations.
- [ ] `msdo/sub-xlayer-duplicate` struck from the roadmap backlog with the
  verification note (or, if a normative home is found, implemented with the
  citation instead).
- [ ] Matrix advances with proof; registry/feature-status/ci gates pass;
  coverage ≥ 90% holds.
