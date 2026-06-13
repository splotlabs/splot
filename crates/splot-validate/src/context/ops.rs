// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Operating-point-set availability and semantics checks.

use super::*;

/// One active in-band operating point set, keyed by `(obu_xlayer_id, ops_id)`
/// (AV2 § 6.10, § 7.3.8.5).
///
/// The parser produces exactly `ops_cnt` operating point payloads (the § 5.10 loop
/// runs `ops_cnt` times or the parse fails), so a separate payload count is redundant
/// and is not stored.
#[derive(Debug, Clone)]
pub(super) struct OperatingPointSetRecord {
    /// `obu_xlayer_id` of the OBU that defined this OPS (`GLOBAL_XLAYER_ID` for a
    /// global OPS).
    pub(super) xlayer_id: ExtendedLayerId,
    /// `ops_id`.
    pub(super) ops_id: u8,
    /// `ops_cnt`, compared against a referencing BRT's `br_ops_cnt` (§ 6.11).
    pub(super) ops_cnt: u8,
    /// Source byte offset of the defining OBU, surfaced in referencing diagnostics.
    pub(super) offset: ByteOffset,
    /// Explicitly signalled `ops_mlayer_info()` entries, retained for the § 6.10.7
    /// dependency-map agreement checks. Inherited and absent entries are not
    /// retained — § 6.10.7 binds the maps "if present", and an inherited entry's
    /// maps are checked when the referenced OPS is itself observed.
    pub(super) explicit_entries: Vec<OpsExplicitEntry>,
}

/// One explicitly signalled `ops_mlayer_info()` entry of an active OPS (§ 5.11.5),
/// retained for the § 6.10.7 dependency-map agreement checks.
#[derive(Debug, Clone)]
pub(super) struct OpsExplicitEntry {
    /// Operating-point payload index (`opIndex`).
    pub(super) payload_index: u8,
    /// The included extended layer (`xLId`) whose configuration the maps describe.
    pub(super) xlayer_id: ExtendedLayerId,
    /// `ops_mlayer_map` plus the per-set-bit `ops_tlayer_map`s.
    pub(super) info: OpsMlayerInfo,
}

/// Collects the explicitly signalled `ops_mlayer_info()` entries of a parsed OPS
/// (§ 5.11.5) for the § 6.10.7 agreement checks.
pub(super) fn ops_explicit_entries(ops: &OperatingPointSet) -> Vec<OpsExplicitEntry> {
    let mut entries = Vec::new();
    for payload in &ops.payloads {
        for entry in &payload.xlayer_entries {
            if let OpsMlayerSource::Explicit(info) = &entry.mlayer {
                entries.push(OpsExplicitEntry {
                    payload_index: payload.index,
                    xlayer_id: entry.xlayer_id,
                    info: info.clone(),
                });
            }
        }
    }
    entries
}

/// Active in-band operating point sets (AV2 § 6.10.1, § 7.3.8.5).
///
/// Unlike [`HlsAvailabilityStore`], this store is **not** monotonic: § 6.10.1 defines
/// explicit reset/update behavior, so records are removed on reset rather than kept
/// forever. State is modeled per extended layer; a global (`GLOBAL_XLAYER_ID`) reset
/// clears every modeled layer.
#[derive(Debug, Default)]
pub(super) struct OpsAvailabilityStore {
    pub(super) by_xlayer: BTreeMap<ExtendedLayerId, BTreeMap<u8, OperatingPointSetRecord>>,
    /// Monotonic count of § 6.10.1 *global* OPS resets (`ops_reset_flag == 1` on a
    /// `GLOBAL_XLAYER_ID` OBU): per § 6.10.1 case 1/2 a global reset resets "all layers
    /// if global", so this generation contributes to the effective reset generation of
    /// *every* extended layer (see [`Self::effective_reset_generation`]).
    pub(super) global_reset_generation: u64,
    /// Per-extended-layer count of § 6.10.1 *local* OPS resets (`ops_reset_flag == 1` on
    /// an OBU with `obu_xlayer_id < GLOBAL_XLAYER_ID`): per § 6.10.1 case 1/2 a local
    /// reset resets only "all OPS for the associated extended layer", so it bumps only
    /// its own layer's generation. The § 6.10.5 buffer-delay sum-constancy error tier
    /// scopes its per-triple baseline by the *effective* reset generation
    /// (`global_reset_generation + local_reset_generation[xlayer]`): a redefinition is
    /// compared against the baseline only when no reset *of that layer* (local or global)
    /// intervened (the constraint says "with no intervening OPS reset"). A reset of an
    /// unrelated extended layer no longer re-baselines this layer (the round-2 fix —
    /// previously a single bitstream-wide counter over-reset every layer and suppressed
    /// a required error). Scoping by the effective generation only ever suppresses
    /// comparisons, never invents one.
    pub(super) local_reset_generation: BTreeMap<ExtendedLayerId, u64>,
    /// Per-`(obu_xlayer_id, opsID)` count of § 6.10.1 *targeted* resets
    /// (`ops_reset_flag == 0` and `ops_cnt == 0`: case 3, "Only OPS x is reset"). A
    /// targeted reset re-baselines exactly that OPS without disturbing any other, so it
    /// must not bump the per-layer effective reset generation (see
    /// [`Self::effective_reset_generation`]) — that would over-suppress unrelated triples
    /// of the same layer. The § 6.10.5 buffer-delay error tier includes
    /// this per-key generation in its scope identity, so a redefinition of the same
    /// triple after a targeted reset of its OPS is treated like any other reset-spanning
    /// change: out of the error tier, into the cross-CVS advisory.
    pub(super) targeted_reset_generation: BTreeMap<(ExtendedLayerId, u8), u64>,
}

impl OpsAvailabilityStore {
    /// Applies one OPS OBU's reset/update semantics (AV2 § 6.10.1):
    ///
    /// | `reset_flag` | `ops_cnt` | behavior |
    /// |---|---|---|
    /// | 1 | 0 | reset all OPS for the layer (all layers if global) |
    /// | 1 | >0 | reset, then define this `(xlayer, ops_id)` |
    /// | 0 | 0 | reset only this `(xlayer, ops_id)` |
    /// | 0 | >0 | define/update only this `(xlayer, ops_id)` |
    pub(super) fn apply(&mut self, record: OperatingPointSetRecord, reset_flag: bool) {
        let xlayer = record.xlayer_id;
        let ops_id = record.ops_id;
        let defines = record.ops_cnt > 0;

        if reset_flag {
            // § 6.10.1 case 1/2: a global reset (GLOBAL_XLAYER_ID) resets "all layers",
            // so it bumps the global generation that every layer's effective generation
            // incorporates; a local reset resets only its own layer's OPS, so it bumps
            // only that layer's generation. Per-layer scoping keeps a reset of one
            // extended layer from re-baselining the § 6.10.5 comparison of another.
            if xlayer.is_global() {
                self.global_reset_generation += 1;
                self.by_xlayer.clear();
            } else {
                *self.local_reset_generation.entry(xlayer).or_default() += 1;
                self.by_xlayer.remove(&xlayer);
            }
            if defines {
                self.by_xlayer
                    .entry(xlayer)
                    .or_default()
                    .insert(ops_id, record);
            }
        } else if defines {
            self.by_xlayer
                .entry(xlayer)
                .or_default()
                .insert(ops_id, record);
        } else {
            // Case 3 (§ 6.10.1): a targeted reset of only this (xlayer, ops_id). Bump the
            // per-key targeted-reset generation so the § 6.10.5 error tier re-baselines
            // this OPS (and only this OPS) like a reset boundary, without touching the
            // per-layer effective reset generation.
            *self
                .targeted_reset_generation
                .entry((xlayer, ops_id))
                .or_default() += 1;
            // Remove only this (xlayer, ops_id), then prune the layer's map if it is
            // now empty so the store does not accumulate empty inner maps.
            let now_empty = match self.by_xlayer.get_mut(&xlayer) {
                Some(map) => {
                    map.remove(&ops_id);
                    map.is_empty()
                }
                None => false,
            };
            if now_empty {
                self.by_xlayer.remove(&xlayer);
            }
        }
    }

    /// Returns the active OPS record for `(xlayer, ops_id)`, if any.
    pub(super) fn get(
        &self,
        xlayer: ExtendedLayerId,
        ops_id: u8,
    ) -> Option<&OperatingPointSetRecord> {
        self.by_xlayer.get(&xlayer).and_then(|map| map.get(&ops_id))
    }

    /// The effective § 6.10.1 reset generation for `xlayer`: the global reset count
    /// (a global reset resets all layers) plus this layer's own local reset count (a
    /// local reset resets only its layer). The § 6.10.5 buffer-delay error tier scopes
    /// a triple's baseline by this value, so only a reset *of this layer* — local or
    /// global — re-baselines its comparison (see [`Self::local_reset_generation`]).
    pub(super) fn effective_reset_generation(&self, xlayer: ExtendedLayerId) -> u64 {
        self.global_reset_generation
            + self
                .local_reset_generation
                .get(&xlayer)
                .copied()
                .unwrap_or(0)
    }

    /// The current § 6.10.1 *targeted*-reset generation for `(xlayer, ops_id)` (see
    /// [`Self::targeted_reset_generation`]), or 0 before any targeted reset of that OPS.
    pub(super) fn targeted_reset_generation(&self, xlayer: ExtendedLayerId, ops_id: u8) -> u64 {
        self.targeted_reset_generation
            .get(&(xlayer, ops_id))
            .copied()
            .unwrap_or(0)
    }

    /// Iterates the active OPS records in the `xlayer` bucket (§ 6.10.7
    /// activation-time re-checks).
    pub(super) fn records_for(
        &self,
        xlayer: ExtendedLayerId,
    ) -> impl Iterator<Item = &OperatingPointSetRecord> {
        self.by_xlayer
            .get(&xlayer)
            .into_iter()
            .flat_map(BTreeMap::values)
    }
}

impl ValidatorContext {
    /// Observes an operating point set OBU: emits the locally-checkable § 6.10
    /// conformance diagnostics and then applies the § 6.10.1 reset/update semantics to
    /// the active OPS state. The local checks run against the *prior* OPS state (before
    /// this OBU is applied) so cross-OPS inheritance references resolve correctly.
    /// Acting is gated on a successful parse and a valid § 5.2.1 extensible tail.
    pub(super) fn observe_operating_point_set(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        let Ok(ops) = parse_operating_point_set(&mut reader, obu.header.extended_layer_id) else {
            return;
        };
        if finish_obu_payload(
            &mut reader,
            obu.payload,
            obu.header.obu_type.is_extensible_obu(),
        )
        .is_err()
        {
            return;
        }

        self.check_operating_point_set_semantics(obu, &ops, report);

        // Annex A.4: OPS-signaled value-space checks (Annex A applies its constraints
        // per sub-bitstream using OPS-derived values, mirror lines 443-451) — a reserved
        // ops_level_idx in 22-30 (Table A.7), and a High ops_tier_flag below level 4.0
        // (Table A.9 NOTE). The OPS PTL carries ops_tier_flag unconditionally, so the
        // high-tier-below-4.0 case is reachable here (unlike the seq-header arm).
        check_ops_level_tier_value_space(obu, &ops, report);

        // AV2 § 6.10.5: the per-(obu_xlayer_id, opsID, op) buffer-delay sum-constancy
        // checks, run before the § 6.10.1 reset/update is applied so the defining OPS's
        // own reset_flag re-baselines its values (the constraint excludes intervening
        // resets) — see check_ops_buffer_delay_sums.
        self.check_ops_buffer_delay_sums(obu, &ops, options, report);

        // AV2 § 6.10.7: explicitly signalled maps are checked against the currently
        // activated sequence headers now, and retained on the record so a later
        // activation can complete the pairing (see on_sequence_activation).
        let explicit_entries = ops_explicit_entries(&ops);
        self.check_ops_entries_against_active(
            obu.offset,
            ops.ops_id,
            &explicit_entries,
            options,
            report,
        );

        // AV2 § 6.10.1: apply reset/update to the active OPS state after the checks.
        let defines = ops.ops_cnt > 0;
        self.ops.apply(
            OperatingPointSetRecord {
                xlayer_id: ops.xlayer_id,
                ops_id: ops.ops_id,
                ops_cnt: ops.ops_cnt,
                offset: obu.offset,
                explicit_entries,
            },
            ops.reset_flag,
        );
        // AV2 § 7.3.8.1: note this OPS (re)send for the random-access-point availability
        // replay, but only when the OBU actually *defines* `(obu_xlayer_id, ops_id)`
        // (`ops_cnt > 0`); a pure reset (`ops_cnt == 0`) makes no OPS available, so it is
        // not a qualifying resend.
        if defines {
            self.rap_replay.note_resend(
                RapHlsKey::OperatingPointSet {
                    xlayer: ops.xlayer_id.get(),
                    ops_id: ops.ops_id,
                },
                obu.header.extended_layer_id,
            );
        }
    }
    /// Emits the locally-checkable § 6.10 OPS conformance diagnostics: local reserved
    /// bits (§ 6.10.2), reserved `ops_mlayer_info_idc` (§ 6.10.2), PTL reserved bits
    /// (§ 6.10.4), `opsBytes` vs `ops_data_size` mismatch (§ 6.10.2), and inherited
    /// operating-point-index bounds (§ 6.10.2).
    pub(super) fn check_operating_point_set_semantics(
        &self,
        obu: &ObuEnvelope<'_>,
        ops: &OperatingPointSet,
        report: &mut ValidationReport,
    ) {
        if ops.has_nonzero_local_reserved_bits() {
            report.push(
                Diagnostic::error(
                    "ops/local-reserved-bits-nonzero",
                    format!(
                        "local operating point set for obu_xlayer_id {} has ops_reserved_2bits {}, \
                         which must be 0",
                        ops.xlayer_id.get(),
                        ops.local_reserved_2bits.unwrap_or(0)
                    ),
                )
                .with_spec_section("6.10.2")
                .with_byte_offset(obu.offset),
            );
        }

        if ops.has_reserved_mlayer_info_idc() {
            report.push(
                Diagnostic::error(
                    "ops/mlayer-info-idc-reserved",
                    format!(
                        "global operating point set {} has ops_mlayer_info_idc == 3, which is \
                         reserved",
                        ops.ops_id
                    ),
                )
                .with_spec_section("6.10.2")
                .with_byte_offset(obu.offset),
            );
        }

        for payload in &ops.payloads {
            if payload.has_size_mismatch() {
                report.push(
                    Diagnostic::error(
                        "ops/payload-size-mismatch",
                        format!(
                            "ops_data_size declares {} byte(s) for OPS {} payload index {}, but \
                             {} byte(s) were parsed",
                            payload.declared_size_bytes,
                            ops.ops_id,
                            payload.index,
                            payload.computed_size_bytes
                        ),
                    )
                    .with_spec_section("6.10.2")
                    .with_byte_offset(obu.offset),
                );
            }

            for entry in &payload.xlayer_entries {
                if let Some(ptl) = &entry.ptl_info
                    && ptl.reserved_2bits != 0
                {
                    report.push(
                        Diagnostic::error(
                            "ops/ptl-reserved-bits-nonzero",
                            format!(
                                "ops_ptl_reserved_2bits is {} for OPS {} payload index {} extended \
                                 layer {}, which must be 0",
                                ptl.reserved_2bits,
                                ops.ops_id,
                                payload.index,
                                entry.xlayer_id.get()
                            ),
                        )
                        .with_spec_section("6.10.4")
                        .with_byte_offset(obu.offset),
                    );
                }

                if let OpsMlayerSource::Inherited {
                    embedded_ops_id,
                    embedded_op_index,
                } = entry.mlayer
                {
                    self.check_inherited_op_index(
                        obu,
                        ops,
                        entry.xlayer_id.get(),
                        embedded_ops_id,
                        embedded_op_index,
                        report,
                    );
                }
            }
        }
    }

    /// Checks an inherited operating-point reference against the § 6.10.2 bounds:
    /// `ops_embedded_op_index < ops_cnt[obu_xlayer_id][refID]`, and — when the
    /// reference is to the current OPS — additionally `ops_embedded_op_index < j` (the
    /// included extended layer). A cross-OPS reference is resolved against the prior
    /// active OPS state; an unresolved cross-OPS reference is not flagged here (it may
    /// be available through external HLS, and the optional
    /// `ops/inherited-ops-unavailable` check is not emitted).
    pub(super) fn check_inherited_op_index(
        &self,
        obu: &ObuEnvelope<'_>,
        ops: &OperatingPointSet,
        xlayer_index: u8,
        ref_ops_id: u8,
        op_index: u8,
        report: &mut ValidationReport,
    ) {
        let out_of_range = if ref_ops_id == ops.ops_id {
            op_index >= ops.ops_cnt || op_index >= xlayer_index
        } else if let Some(referenced) = self.ops.get(ops.xlayer_id, ref_ops_id) {
            op_index >= referenced.ops_cnt
        } else {
            return;
        };

        if out_of_range {
            report.push(
                Diagnostic::error(
                    "ops/inherited-op-index-out-of-range",
                    format!(
                        "OPS {} payload extended layer {} inherits from ops_embedded_ops_id {} \
                         ops_embedded_op_index {}, which is out of range for the referenced \
                         operating point set",
                        ops.ops_id, xlayer_index, ref_ops_id, op_index
                    ),
                )
                .with_spec_section("6.10.2")
                .with_byte_offset(obu.offset),
            );
        }
    }

    /// Checks explicitly signalled OPS maps against the sequence header activated
    /// for each entry's extended layer (AV2 § 6.10.7): for any included embedded
    /// layer `cMId` with `MLayerDependencyMap[cMId][rMId] == 1`, embedded layer
    /// `rMId` must also be included, and likewise per temporal-layer map under
    /// `TLayerDependencyMap`. An entry whose extended layer has no decidable
    /// activated in-band sequence header is skipped (the maps are never
    /// fabricated or guessed; see `agreement_activation_for`), the whole check
    /// is suppressed when external HLS declares any sequence header, and the
    /// [`DependencyFindingKey`] dedup makes activation-time re-checks
    /// idempotent.
    pub(super) fn check_ops_entries_against_active(
        &mut self,
        ops_offset: ByteOffset,
        ops_id: u8,
        entries: &[OpsExplicitEntry],
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if external_declares_sequence_header(options) {
            return;
        }
        for entry in entries {
            let Some((seq_header_id, general)) = self.agreement_activation_for(entry.xlayer_id)
            else {
                continue;
            };

            if let Some((curr, reference)) =
                mlayer_closure_violation(entry.info.mlayer_map, &general.mlayer_dependency_map)
            {
                let key = DependencyFindingKey::Ops {
                    ops_offset,
                    payload_index: entry.payload_index,
                    entry_xlayer: entry.xlayer_id,
                    seq_header_id,
                    map: DependencyMapKind::Mlayer,
                };
                if self.emitted_dependency_findings.insert(key) {
                    report.push(
                        Diagnostic::error(
                            "ops/mlayer-dependency-missing",
                            format!(
                                "OPS {ops_id} operating point {} for extended layer {} includes \
                                 embedded layer {curr} but not embedded layer {reference}, which \
                                 the activated sequence header {}'s \
                                 MLayerDependencyMap[{curr}][{reference}] requires",
                                entry.payload_index,
                                entry.xlayer_id.get(),
                                seq_header_id.get(),
                            ),
                        )
                        .with_spec_section("6.10.7")
                        .with_byte_offset(ops_offset),
                    );
                }
            }

            for &(mlayer, tlayer_mask) in &entry.info.tlayer_maps {
                let Some((curr, reference)) =
                    tlayer_closure_violation(mlayer, tlayer_mask, &general.tlayer_dependency_map)
                else {
                    continue;
                };
                let key = DependencyFindingKey::Ops {
                    ops_offset,
                    payload_index: entry.payload_index,
                    entry_xlayer: entry.xlayer_id,
                    seq_header_id,
                    map: DependencyMapKind::Tlayer { mlayer },
                };
                if self.emitted_dependency_findings.insert(key) {
                    report.push(
                        Diagnostic::error(
                            "ops/tlayer-dependency-missing",
                            format!(
                                "OPS {ops_id} operating point {} for extended layer {} includes \
                                 temporal layer {curr} of embedded layer {mlayer} but not \
                                 temporal layer {reference}, which the activated sequence header \
                                 {}'s TLayerDependencyMap[{mlayer}][{curr}][{reference}] requires",
                                entry.payload_index,
                                entry.xlayer_id.get(),
                                seq_header_id.get(),
                            ),
                        )
                        .with_spec_section("6.10.7")
                        .with_byte_offset(ops_offset),
                    );
                }
            }
        }
    }
}
