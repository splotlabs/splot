// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Film-grain state, diagnostics, and frame-reference checks.

use super::*;

/// Maximum conformant `num_*_points` for a film-grain scaling function
/// (AV2 v1.0.0 § 6.17.10.2).
pub(super) const MAX_FILM_GRAIN_SCALING_POINTS: u8 = 14;

/// Per-slot film-grain availability, recorded when a film-grain OBU updates a slot
/// (AV2 § 6.13 / § 7.3.8 foundation). Frame-reference checks use the stored layer and
/// chroma identity; duplicate-slot diagnostics cite the conflicting update.
#[derive(Debug, Clone, Copy)]
pub(super) struct FgmSlotRecord {
    /// `FgmChromaIdc[slot]` (the defining film-grain OBU's `fgm_chroma_idc`, sharing the
    /// `chroma_format_idc` value space — AV2 § 6.17.10.1 requires equality).
    pub(super) chroma_idc: u32,
    /// `FgmMLayerId[slot]` — the embedded layer of the film-grain OBU that defined the slot.
    pub(super) mlayer_id: EmbeddedLayerId,
    /// `FgmTLayerId[slot]` — the temporal layer of the film-grain OBU that defined the slot.
    pub(super) tlayer_id: TemporalLayerId,
}

/// Film-grain state (§ 6.13): duplicate updates reset at each coded-frame boundary;
/// per-slot availability remains for frame-reference checks (§ 7.3.8).
#[derive(Debug, Default)]
pub(super) struct FilmGrainState {
    /// Slots (`fgm_update_flags` bits) updated by a film-grain OBU since the last
    /// coded frame unit.
    pub(super) updated_slots_since_coded_frame: u8,
    /// Monotonic per-slot availability for frame-reference validation.
    pub(super) available: [Option<FgmSlotRecord>; MAX_FILM_GRAIN],
}

impl FilmGrainState {
    /// Clears the §6.13 coded-frame-unit window at a coded-frame boundary.
    pub(super) fn reset_coded_frame_window(&mut self) {
        self.updated_slots_since_coded_frame = 0;
    }
}

/// Checks ascending scaling-point values for each film-grain component (§ 6.17.10.2).
pub(super) fn emit_scaling_point_order_diagnostics(
    channel: &str,
    points: &[FilmGrainScalingPoint],
    slot: u8,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    for i in 1..points.len() {
        let value = points[i].value;
        if value <= points[i - 1].value || value >= 256 {
            report.push(
                Diagnostic::error(
                    "film-grain/scaling-point-not-increasing",
                    format!(
                        "film grain slot {slot} {channel} scaling point {i} value {value} must be \
                         strictly greater than the previous point and less than 256"
                    ),
                )
                .with_spec_section("6.17.10.2")
                .with_byte_offset(obu.offset),
            );
        }
    }
}

/// Checks film-grain model references against linear in-band availability. External
/// HLS suppresses absence judgments; RAP resend requirements are checked separately.
pub(super) fn frame_film_grain_reference_checks(
    film_grain: splot_core::headers::frame::FilmGrainConfig,
    film_grain_state: &FilmGrainState,
    active_sequence: &SequenceHeader,
    options: &ValidationOptions,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) -> Option<u8> {
    if !film_grain.apply_grain {
        return None;
    }
    if !matches!(options.external_hls, ExternalHlsMode::Disabled) {
        return None;
    }
    let fgm_id = film_grain.fgm_id?;
    let slot = usize::from(fgm_id);
    let slot_record = film_grain_state.available.get(slot)?;
    let Some(record) = slot_record else {
        report.push(frame_header_error(
            "frame-header/film-grain-model-unavailable",
            "6.17.10.1",
            obu,
            format!(
                "film_grain_config() has apply_grain == 1 and references fgm_id {fgm_id}, but no \
                 film grain OBU has set FilmGrainPresent[{fgm_id}] == 1 (no received model for \
                 that slot)"
            ),
        ));
        return None;
    };

    let general = &active_sequence.general;
    let frame_mlayer = obu.header.embedded_layer_id;
    let frame_tlayer = obu.header.temporal_layer_id;

    if !general
        .mlayer_dependency_map
        .depends_on(frame_mlayer, record.mlayer_id)
    {
        report.push(frame_header_error(
            "frame-header/film-grain-mlayer-dependency-missing",
            "6.17.10.1",
            obu,
            format!(
                "film_grain_config() at obu_mlayer_id {} references fgm_id {fgm_id} whose film \
                 grain model was defined at embedded layer {}, but the active sequence header's \
                 MLayerDependencyMap[{}][{}] is 0 (§ 6.17.10.1)",
                frame_mlayer.get(),
                record.mlayer_id.get(),
                frame_mlayer.get(),
                record.mlayer_id.get(),
            ),
        ));
    }

    if !general
        .tlayer_dependency_map
        .depends_on(frame_mlayer, frame_tlayer, record.tlayer_id)
    {
        report.push(frame_header_error(
            "frame-header/film-grain-tlayer-dependency-missing",
            "6.17.10.1",
            obu,
            format!(
                "film_grain_config() at obu_tlayer_id {} references fgm_id {fgm_id} whose film \
                 grain model was defined at temporal layer {}, but the active sequence header's \
                 TLayerDependencyMap[{}][{}][{}] is 0 (§ 6.17.10.1)",
                frame_tlayer.get(),
                record.tlayer_id.get(),
                frame_mlayer.get(),
                frame_tlayer.get(),
                record.tlayer_id.get(),
            ),
        ));
    }

    let chroma_format_idc = u32::from(general.chroma_format_idc.get());
    if record.chroma_idc <= 3 && record.chroma_idc != chroma_format_idc {
        report.push(frame_header_error(
            "frame-header/film-grain-chroma-idc-mismatch",
            "6.17.10.1",
            obu,
            format!(
                "film_grain_config() references fgm_id {fgm_id} whose film grain model has \
                 FgmChromaIdc {} but the active sequence header's chroma_format_idc is {} \
                 (§ 6.17.10.1)",
                record.chroma_idc, chroma_format_idc,
            ),
        ));
    }

    Some(fgm_id)
}

impl ValidatorContext {
    /// Observes a film grain OBU (§ 5.14), running the locally-checkable § 6.13
    /// diagnostics (zero update flags, out-of-range chroma idc, duplicate slot in the
    /// coded frame unit) and recording per-slot availability. A parse failure or
    /// malformed payload tail is silent, consistent with the OPS observer.
    pub(super) fn observe_film_grain(
        &mut self,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        let Ok(fg) = parse_film_grain(&mut reader) else {
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
        self.emit_film_grain_diagnostics(obu, &fg, report);
        Self::emit_film_grain_model_diagnostics(obu, &fg, report);
        self.record_film_grain(obu, &fg);
    }

    /// Emits the locally-decidable § 6.17.10.2 film-grain *model* conformance
    /// diagnostics for each updated slot: scaling-point counts (`num_*_points <= 14`),
    /// strictly-increasing-and-`< 256` scaling-point values, and the 4:2:0 chroma
    /// pairing rule (when `subX == 1 && subY == 1`, `num_cb_points` and `num_cr_points`
    /// must be both zero or both non-zero).
    pub(super) fn emit_film_grain_model_diagnostics(
        obu: &ObuEnvelope<'_>,
        fg: &FilmGrainObu,
        report: &mut ValidationReport,
    ) {
        for update in &fg.models {
            let model = &update.model;
            let slot = update.slot;
            for (channel, count) in [
                ("y", model.num_y_points),
                ("cb", model.num_cb_points),
                ("cr", model.num_cr_points),
            ] {
                if count > MAX_FILM_GRAIN_SCALING_POINTS {
                    report.push(
                        Diagnostic::error(
                            "film-grain/scaling-points-out-of-range",
                            format!(
                                "film grain slot {slot} num_{channel}_points {count} must be less \
                                 than or equal to {MAX_FILM_GRAIN_SCALING_POINTS}"
                            ),
                        )
                        .with_spec_section("6.17.10.2")
                        .with_byte_offset(obu.offset),
                    );
                }
            }

            for (channel, points) in [
                ("y", &model.point_y),
                ("cb", &model.point_cb),
                ("cr", &model.point_cr),
            ] {
                emit_scaling_point_order_diagnostics(channel, points, slot, obu, report);
            }

            // AV2 § 6.17.10.2: in 4:2:0 (subX == 1 && subY == 1), film grain applies to
            // both chroma components or neither.
            if fg.sub_x && fg.sub_y && (model.num_cb_points == 0) != (model.num_cr_points == 0) {
                report.push(
                    Diagnostic::error(
                        "film-grain/chroma-points-not-paired",
                        format!(
                            "film grain slot {slot}: in 4:2:0, num_cb_points ({}) and \
                             num_cr_points ({}) must both be zero or both non-zero",
                            model.num_cb_points, model.num_cr_points
                        ),
                    )
                    .with_spec_section("6.17.10.2")
                    .with_byte_offset(obu.offset),
                );
            }
        }
    }

    /// Emits the § 6.13 film-grain diagnostics for `fg`, reading the coded-frame-unit
    /// window and per-slot availability captured before this OBU.
    pub(super) fn emit_film_grain_diagnostics(
        &self,
        obu: &ObuEnvelope<'_>,
        fg: &FilmGrainObu,
        report: &mut ValidationReport,
    ) {
        if fg.update_flags == 0 {
            report.push(
                Diagnostic::error(
                    "film-grain/update-flags-zero",
                    "fgm_update_flags must not be 0",
                )
                .with_spec_section("6.13")
                .with_byte_offset(obu.offset),
            );
        }

        if fg.chroma_idc > 3 {
            report.push(
                Diagnostic::error(
                    "film-grain/chroma-idc-out-of-range",
                    format!(
                        "fgm_chroma_idc {} must be less than or equal to 3",
                        fg.chroma_idc
                    ),
                )
                .with_spec_section("6.13")
                .with_byte_offset(obu.offset),
            );
        }

        let overlap = self.film_grain.updated_slots_since_coded_frame & fg.update_flags;
        for slot in 0..MAX_FILM_GRAIN {
            if overlap & (1 << slot) == 0 {
                continue;
            }
            let prior = match self.film_grain.available[slot] {
                Some(record) => format!(
                    " (previously updated by a film grain OBU at embedded layer {}, temporal \
                     layer {}, fgm_chroma_idc {})",
                    record.mlayer_id.get(),
                    record.tlayer_id.get(),
                    record.chroma_idc,
                ),
                None => String::new(),
            };
            report.push(
                Diagnostic::error(
                    "film-grain/duplicate-slot-in-coded-frame-unit",
                    format!(
                        "film grain slot {slot} is updated more than once in the same coded \
                         frame unit{prior}"
                    ),
                )
                .with_spec_section("6.13")
                .with_byte_offset(obu.offset),
            );
        }
    }

    /// Updates the §6.13 coded-frame-unit window and per-slot availability, and records each
    /// updated slot as a § 7.3.8.1 random-access-point replay (re)send event (AV2 § 7.3.8.8:
    /// a film-grain OBU defines a model that a later frame may reference; the replay catches a
    /// model sent before a random access point and not resent in or after it).
    pub(super) fn record_film_grain(&mut self, obu: &ObuEnvelope<'_>, fg: &FilmGrainObu) {
        self.film_grain.updated_slots_since_coded_frame |= fg.update_flags;
        for update in &fg.models {
            let index = update.slot as usize;
            if index < MAX_FILM_GRAIN {
                self.film_grain.available[index] = Some(FgmSlotRecord {
                    chroma_idc: fg.chroma_idc,
                    mlayer_id: obu.header.embedded_layer_id,
                    tlayer_id: obu.header.temporal_layer_id,
                });
                self.rap_replay.note_resend(
                    RapHlsKey::FilmGrain { slot: update.slot },
                    obu.header.extended_layer_id,
                );
            }
        }
    }
}
