# Tasks: MSDO sub-stream constraint checks

## 1. Pre-implementation bookkeeping

- [x] 1.1 Matrix: `openspec_change = "msdo-substream-constraint-checks"` on
  `AV2-5.6-MSDO`; register the change in `openspec/changes/README.md`.
- [x] 1.2 Verify what § 6.6 checks are already landed (layer-id rules,
  `num_streams_minus_2 <= 2`) so nothing is duplicated; list them in the
  proposal-tracking notes.

## 2. Locally-decidable MSDO checks

- [x] 2.1 `msdo/profile-below-substream-max` (error, § 6.6 mirror line 1347)
  at the MSDO OBU.
- [x] 2.2 `annex-a/profile-reserved` for reserved `multistream_profile_idc`
  (§ 6.6 value-space sentence; note the Table A.4→A.1 erratum in the doc
  comment).

## 3. Substream-max agreement checks (stateful)

- [x] 3.1 Record the latest MSDO's `sub_xlayer_id[i] → (max_profile,
  max_level, max_tier)` mapping in validator state (reusable by the
  Table A.4 re-land).
- [x] 3.2 `msdo/substream-profile-exceeds-max` / `-level-` / `-tier-`
  (errors, § 6.6 lines 1365–1378): evaluated on frame-confirmed sequence
  activation against the recorded mapping AND on MSDO arrival against
  already-confirmed activations; both arrival orders tested. Equality
  passes (boundary tests).

## 4. DOH-constraint flag check

- [x] 4.1 `msdo/doh-constraint-required` (error, § 6.6 line 1391): any
  frame-confirmed activated header of the CMVS with
  `monotonic_output_order_flag == 0` while the recorded MSDO has
  `multistream_doh_constraint_flag == 0`; gate on `CmvsState::Inside` like
  the landed monotonic agreement check; both arrival orders.

## 5. § 7.3.8.2 non-RAP identity

- [x] 5.1 Track temporal-unit RAP-ness (§ 7.4.1: TU contains CLK/OLK/RAS)
  and buffer the TU's MSDO payload fingerprint; at TU end, a non-RAP TU
  whose MSDO fingerprint differs from the previous MSDO's →
  `msdo/non-rap-not-identical` (error, § 7.3.8.2 mirror line 716). RAP TUs
  update the reference fingerprint without checking. Reuse the existing
  payload-fingerprint helper.
- [x] 5.2 Correct the `AV2-5.6-MSDO` matrix note (§ 7.3.8.2 is
  OBU-type-detectable per § 7.4.1, not frame-activation-blocked).

## 6. Roadmap hygiene

- [x] 6.1 Re-verify § 6.6 mirror lines 1330–1398 contain no `sub_xlayer_id`
  uniqueness requirement; strike the planned `msdo/sub-xlayer-duplicate`
  backlog row from `docs/VALIDATOR-ROADMAP.md` with the verification note
  (or implement it with a real citation if a normative home exists).

## 7. Docs, registry, generated artifacts

- [x] 7.1 Register all new ids in `docs/VALIDATOR-DIAGNOSTICS.md`; `msdo/`
  namespace added to `DIAGNOSTIC_PREFIXES` if not present.
- [x] 7.2 Matrix advances with proof; regenerate FEATURE-STATUS/
  SPEC-COVERAGE.

## 8. Verification

- [x] 8.1 Tests per acceptance criteria (both arrival orders, boundary
  equality, RAP exemption, identical-MSDO pass, ExternalHlsMode handling
  consistent with existing msdo-adjacent checks).
- [x] 8.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 8.3 `cargo xtask ci` passes (run bare, exit checked).
