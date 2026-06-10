# Tasks — layer-dependency-map agreement checks

## 1. State plumbing

- [x] 1.1 Extend `OperatingPointSetRecord` with the explicitly signalled
      per-entry maps (`payload_index`, entry `xlayer_id`, `mlayer_map`,
      per-set-bit tlayer maps), populated in `observe_operating_point_set`;
      § 6.10.1 reset/update semantics unchanged (regression-covered by
      existing OPS state tests).
- [x] 1.2 Extend the HLS store's LCR records with parsed embedded-layer info:
      `global_id -> per-xId` info and `(xlayer, local_id) -> info`, populated
      in `observe_layer_config_record` at the existing recording points.

## 2. Checks

- [x] 2.1 Add the shared dependency-closure helper over
      `MLayerDependencyMap::depends_on` / `TLayerDependencyMap::depends_on`
      (design D1), with unit coverage for the closure predicate itself.
- [x] 2.2 OPS § 6.10.7: observation-side check
      (`check_ops_entries_against_active`, called from
      `observe_operating_point_set` alongside the existing semantics checks;
      explicit entries only, per-entry activated header, external-HLS
      suppression) emitting `ops/mlayer-dependency-missing` /
      `ops/tlayer-dependency-missing`.
- [x] 2.3 OPS § 6.10.7: activation-side re-check when
      `active_sequence_by_xlayer` is newly set or changes (both activation
      points), with the D3 dedup keys; a same-id sequence-header redefinition
      that changes the agreement inputs invalidates the id's keys and re-fires
      the checks.
- [x] 2.4 LCR § 6.8.9: activation-time pairing via the `seq_lcr_id`
      local-then-global resolution, checking the `xId == x` entry and emitting
      `lcr/mlayer-dependency-missing` / `lcr/tlayer-dependency-missing` at the
      LCR OBU offset, with dedup. A later-arriving LCR is deliberately not
      retroactively paired (§ 6.4.1 "present prior to this sequence header"),
      redefinitions replace the stored maps wholesale, and the check is
      suppressed under any `ExternalHlsMode::Provided`.
- [x] 2.5 MFH § 7.3.8.7 / § 6.17.2: two-predicate check in
      `resolve_frame_header_reference`'s `cur_mfh_id > 0` branch emitting
      `frame-header/mfh-mlayer-dependency-missing` /
      `frame-header/mfh-tlayer-dependency-missing`; remove the resolved
      `TODO(spec: AV2-5.7-MULTI-FRAME-HEADER)`.

## 3. Tests (positive, negative, ordering, suppression)

- [x] 3.1 OPS: negative (mlayer and tlayer closure violations), positive
      (closed maps silent), OPS-before-activation ordering (checked exactly
      once), frame-driven re-activation emission, same-id redefinition
      re-emission, no-activated-header silence, external-HLS suppression,
      inherited entries not checked (discriminating: the inherited source's
      own violation fires exactly once).
- [x] 3.2 LCR: negative coverage of all four `isGlobal` × map cells, positive
      silence, local-over-global precedence, `seq_lcr_id == 0` and
      unresolved-nonzero silence, late-LCR non-pairing, redefinition-clears-
      maps silence, Provided-mode suppression, no duplicate across repeated
      activation, LCR offset on the diagnostic.
- [x] 3.3 MFH: negative mlayer and tlayer predicate failures, positive
      silence, unavailable-MFH / external-only / unresolvable-sequence-header
      silence.
- [x] 3.4 Regression: existing diagnostic ids/severities/sections unchanged
      (full validator suite passes untouched).

## 4. Docs, registry, matrix

- [x] 4.1 Add the six rule rows to `docs/VALIDATOR-DIAGNOSTICS.md`; replace
      the "Deferred pending infrastructure" non-check note with the precise
      scope of what is now checked (and what still is not).
- [x] 4.2 Remove the two landed backlog rows from
      `docs/VALIDATOR-ROADMAP.md`; refresh the Phase 6 status sentence.
- [x] 4.3 Update `docs/IMPLEMENTATION-MATRIX.toml`: notes + `diagnostics`
      proofs for `AV2-5.10-OPERATING-POINT-SET` (and the § 5.11 payload row
      owning `ops_mlayer_info`), `AV2-5.8-LAYER-CONFIG-RECORD` +
      `AV2-5.8.8-LCR-EMBEDDED-LAYER-INFO`, `AV2-5.7-MULTI-FRAME-HEADER`;
      `validate` stages stay `partial` with honest remaining-work notes.
- [x] 4.4 Update the validator capability spec via the delta (this change's
      `specs/validator/spec.md`).

## 5. Gates

- [x] 5.1 `cargo xtask feature-status` and `cargo xtask check-feature-status`
      pass.
- [x] 5.2 `cargo xtask ci` passes.
- [x] 5.3 `cargo xtask audit-scope --all --write-ledger` re-recorded after the
      tracked-file edits.
