# Proposal: MSDO/global-LCR agreement, cmvs/* diagnostics, Table A.4 re-land

## Feature IDs

- `AV2-5.8-LAYER-CONFIG-RECORD`, `AV2-5.8.1-LCR-GLOBAL-INFO` (§ 6.8.2)
- `AV2-7.3.2-CMVS-BOUNDARIES` (the first `cmvs/` diagnostics)
- `AV2-A-PROFILES` (Table A.4 IOP presence re-land)

## Why

Three related gaps share state: (1) the § 6.8.2 MSDO↔global-LCR agreement
sentences (mirror lines 1646–1678) are unchecked although both records are
parsed and the activation chain (`seq_lcr_id` → local LCR `lcr_global_id` →
global record) is modeled; (2) `AV2-7.3.2-CMVS-BOUNDARIES` still has no
`cmvs/` diagnostics; (3) the Table A.4 IOP-presence checks descoped from
PR #46 now have their prerequisites: MSDO aggregate-profile state landed with
PR #47 (`multistream_profile_idc` recorded), LCR activation is decidable via
the association chain, and the § 7.3.6 per-TU attribution requirements are
recorded in PR #46's codex threads (3392613056/59/68, 3392534625).

## What Changes

Grounded in `06-syntax-structures-semantics.md#s-6-8-2` (lines 1615–1678),
`07-decoding-process.md#s-7-3-2` (lines 320–364), and Annex A.2/A.3 (Tables
A.1/A.5/A.6) + Table A.4 (lines 173–201):

1. **§ 6.8.2 agreement** (when an OBU_MSDO and an *activated* global LCR are
   present in the same CMVS; evaluated at CMVS-membership resolution like the
   landed DOH check):
   - `lcr/msdo-stream-count-mismatch` (error): `num_streams_minus_2 + 2 !=
     LcrMaxNumXLayerCount`.
   - `lcr/msdo-sub-xlayer-not-in-lcr` (error): a `sub_xlayer_id[i]` not in
     `LcrXLayerID[]`.
   - `lcr/msdo-aggregate-mismatch` (error, field named in the message): with
     `lcr_aggregate_info_present_flag == 1` — `multistream_profile_idc`
     inconsistent with `lcr_config_idc` per Annex A.3 Table A.6; the Table
     A.1 interoperability point of `multistream_profile_idc` !=
     `lcr_max_interop`; `multistream_level_idx != lcr_aggregate_level_idx`;
     `multistream_tier != lcr_max_tier_flag`.
   - `lcr/msdo-substream-ptl-mismatch` (error): with
     `lcr_seq_profile_tier_level_info_present_flag == 1`,
     `sub_stream_max_*[i] != lcr_*[sub_xlayer_id[i]]` equality per § 6.8.2.
   - `lcr/msdo-doh-flag-mismatch` (error): `multistream_doh_constraint_flag
     != lcr_doh_constraint_flag`.
2. **§ 6.8.2 DOH requirement**: `lcr/doh-constraint-required` (error, line
   1619) — `monotonic_output_order_flag == 0` in any frame-confirmed
   activated header of the CMVS while the activated global LCR has
   `lcr_doh_constraint_flag == 0`; same deferred-resolution mechanism as
   `msdo/doh-constraint-required`.
3. **§ 7.3.2 boundary identity**: `cmvs/boundary-set-mismatch` (error, 07
   mirror line ~351): the MSDO-derived CMVS boundary set must equal the
   MSDO+LCR-derived set — emitted only on decidable disagreement (the
   CmvsTracker's Unknown states stay silent). First id in the `cmvs/`
   namespace (add to `DIAGNOSTIC_PREFIXES`).
4. **Table A.4 re-land**: `annex-a/msdo-required-for-iop`,
   `annex-a/lcr-required-for-iop`, `annex-a/msdo-prohibited-for-iop`
   (documented defensive arm), with ALL the PR #46 review requirements:
   - IOP from the MSDO's `multistream_profile_idc` (Table A.1) when an MSDO
     is in the window, else from frame-confirmed activated headers;
   - only *activated* global LCRs satisfy the global-LCR arms;
   - per-TU observation attribution: a TU containing a CLK belongs to the
     NEW CVS (§ 7.3.6), so pre-CLK HLS in that TU seeds the new window, not
     the old one;
   - same-id CLK reactivations seed windows from the active confirmed
     header; windows span the whole CVS (flush at next CVS start or EOS);
   - extended-layer counting per the Table A.3 definition order
     (MSDO-declared, then activated-global-LCR `LcrMaxNumXLayerCount`, then
     observed distinct xlayers).
   The `interoperability_point` helper returns to `annex_a.rs` with Table
   A.6 (multi-sequence configuration value spaces) transcribed alongside.

## Non-goals

- Full § 7.3.8.3 LCR availability modeling (begin/end clause cases the
  CmvsTracker routes to Unknown stay Unknown).
- `lcr_enforce_tile_alignment_flag` cross-layer tile-structure checks
  (frame/tile-blocked; record as `TODO(spec: AV2-5.8.1-LCR-GLOBAL-INFO)` if
  not already noted).
- § 6.8.5 PTL-vs-activated-header and § 6.8.8 rep-info agreement (next
  backlog item, `lcr-ptl-activated-sequence-agreement`).
- The § 7.3.7 DOH constraints themselves (`celu-orderhint-constraints`).

## Acceptance criteria

- [x] Every § 6.8.2 constraint group has positive/negative/boundary tests in
  both arrival orders, with activation-gated global-LCR resolution (an
  observed-but-never-activated global LCR triggers nothing).
- [x] Table A.6/A.1 transcriptions verified cell-by-cell against the mirror.
- [x] Table A.4 re-land covers every PR #46 codex-thread scenario as a test:
  pre-CLK MSDO attribution, same-id reactivation, late-TU second xlayer,
  unactivated global LCR not satisfying the arm, declared-count precedence.
- [x] Unknown CMVS/activation states never fire (under-approximation tests).
- [x] Matrix rows advance with proof; registry/feature-status/ci/coverage
  gates pass.
