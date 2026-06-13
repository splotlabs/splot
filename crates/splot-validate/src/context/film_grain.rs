// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Film-grain state, diagnostics, and frame-reference checks.

use super::*;

/// Maximum conformant `num_*_points` for a film-grain scaling function
/// (AV2 v1.0.0 § 6.17.10.2).
pub(super) const MAX_FILM_GRAIN_SCALING_POINTS: u8 = 14;

/// Per-slot film-grain availability, recorded when a film-grain OBU updates a slot
/// (AV2 § 6.13 / § 7.3.8 foundation). Kept for future frame-reference checks; this
/// phase reads it only to cite the conflicting update in a duplicate-slot diagnostic.
#[derive(Debug, Clone, Copy)]
pub(super) struct FgmSlotRecord {
    /// `FgmChromaIdc[slot]`.
    pub(super) chroma_idc: u32,
    /// `FgmMLayerId[slot]`.
    pub(super) mlayer_id: u8,
    /// `FgmTLayerId[slot]`.
    pub(super) tlayer_id: u8,
}

/// Film-grain validator state (AV2 § 6.13).
///
/// `updated_slots_since_coded_frame` resets at each coded-frame-unit boundary (see
/// [`ValidatorContext::reset_coded_frame_window`]) and drives the § 6.13 duplicate-slot
/// check. The `available` array is monotonic per-slot HLS state, foundation for the
/// deferred frame film-grain-reference checks (`apply_grain` / `fgm_id`, § 5.18.10.1 /
/// § 7.3.8).
#[derive(Debug, Default)]
pub(super) struct FilmGrainState {
    /// Slots (`fgm_update_flags` bits) updated by a film-grain OBU since the last
    /// coded frame unit.
    pub(super) updated_slots_since_coded_frame: u8,
    /// Monotonic per-slot availability for future frame-reference validation.
    pub(super) available: [Option<FgmSlotRecord>; MAX_FILM_GRAIN],
}

impl FilmGrainState {
    /// Clears the §6.13 coded-frame-unit window at a coded-frame boundary.
    pub(super) fn reset_coded_frame_window(&mut self) {
        self.updated_slots_since_coded_frame = 0;
    }
}

/// Returns `true` if `obu_type`'s payload begins with a `frame_header()` or
/// `tile_group_obu()` (AV2 v1.0.0 § 5.2.1): the tile-group types, plus the SEF / TIP
/// / bridge frames that call `frame_header( 1 )` directly.
/// Emits `film-grain/scaling-point-not-increasing` for any scaling point whose
/// (cumulative) value is not strictly greater than its predecessor or is not less than
/// 256 (AV2 v1.0.0 § 6.17.10.2: for `i > 0`, `point_*_value[i] > point_*_value[i - 1]`
/// and `< 256`).
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

/// Emits the locally-decidable § 6.17.10.1 / § 7.3.8.8 film-grain *availability*
/// diagnostic for a parsed `film_grain_config()`: when `apply_grain == 1`, the referenced
/// `fgm_id` slot must have a received film-grain model (`FilmGrainPresent[ fgm_id ] == 1`).
///
/// **Scope and under-reporting (zero-false-positive discipline, AGENTS.md § 7).** This
/// covers ONLY the `FilmGrainPresent[ fgm_id ] == 1` requirement of § 6.17.10.1, and only
/// the in-band-availability half:
///
/// - **External means.** § 7.3.8.8 allows the model to be available "by provision through
///   external means". [`ExternalHlsSet`](crate::options::ExternalHlsSet) cannot express
///   film-grain OBUs (only sequence headers and operating point sets), so under any
///   `ExternalHlsMode::Provided` the model MAY be external without being listed — exactly
///   the inexpressible-kind case the blanket "any Provided suppresses" policy covers. The
///   check therefore fires only under `ExternalHlsMode::Disabled`.
/// - **Random-access-point visibility (§ 7.3.8.1).** A model available only from an earlier
///   position is unavailable at a later random access point that drops it. `available[]` is
///   monotonic (never reset at a random access point), so this check OVER-approximates
///   presence and silently UNDER-reports that random-access-point-unavailability direction.
///   That is a named residual on AV2-7.3.8-HLS-AVAILABILITY (no random-access-point replay
///   for film-grain references yet), not a false positive: the linear absence test can only
///   miss findings, never invent them. The companion § 6.17.10.1 layer-dependency
///   constraints (FgmTLayerId / FgmMLayerId / FgmChromaIdc) also remain a residual.
///
/// A `None`-for-the-slot under `Disabled` is therefore decidable and sound: no in-band film
/// grain OBU ever set the slot before this frame and no external provision is possible.
pub(super) fn frame_film_grain_reference_checks(
    film_grain: &splot_core::headers::frame::FilmGrainConfig,
    film_grain_state: &FilmGrainState,
    options: &ValidationOptions,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    // The fgm_id reference (and its § 6.17.10.1 requirement) exists only when apply_grain.
    if !film_grain.apply_grain {
        return;
    }
    // Film grain OBUs cannot be expressed by ExternalHlsSet, so under any Provided mode the
    // referenced model MAY be supplied externally without being listed — suppress to avoid a
    // false positive. Only the external-disabled case is decidable from the bitstream alone.
    if !matches!(options.external_hls, ExternalHlsMode::Disabled) {
        return;
    }
    let Some(fgm_id) = film_grain.fgm_id else {
        return;
    };
    let slot = usize::from(fgm_id);
    // A slot outside the modeled range cannot be matched against availability state; the
    // fgm_id field is f(3) (0..=7), so this never trips for a parsed config, but guard it.
    let Some(record) = film_grain_state.available.get(slot) else {
        return;
    };
    if record.is_none() {
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
    }
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
        self.emit_film_grain_model_diagnostics(obu, &fg, report);
        self.record_film_grain(obu, &fg);
    }

    /// Emits the locally-decidable § 6.17.10.2 film-grain *model* conformance
    /// diagnostics for each updated slot: scaling-point counts (`num_*_points <= 14`),
    /// strictly-increasing-and-`< 256` scaling-point values, and the 4:2:0 chroma
    /// pairing rule (when `subX == 1 && subY == 1`, `num_cb_points` and `num_cr_points`
    /// must be both zero or both non-zero).
    pub(super) fn emit_film_grain_model_diagnostics(
        &self,
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
        // AV2 § 6.13: fgm_update_flags is not equal to 0.
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

        // AV2 § 6.13: fgm_chroma_idc is less than or equal to 3.
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

        // AV2 § 6.13: bit i of fgm_update_flags is set in at most one film grain OBU
        // per coded frame unit.
        let overlap = self.film_grain.updated_slots_since_coded_frame & fg.update_flags;
        for slot in 0..MAX_FILM_GRAIN {
            if overlap & (1 << slot) == 0 {
                continue;
            }
            let prior = match self.film_grain.available[slot] {
                Some(record) => format!(
                    " (previously updated by a film grain OBU at embedded layer {}, temporal \
                     layer {}, fgm_chroma_idc {})",
                    record.mlayer_id, record.tlayer_id, record.chroma_idc,
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

    /// Updates the §6.13 coded-frame-unit window and per-slot availability.
    pub(super) fn record_film_grain(&mut self, obu: &ObuEnvelope<'_>, fg: &FilmGrainObu) {
        self.film_grain.updated_slots_since_coded_frame |= fg.update_flags;
        for update in &fg.models {
            let index = update.slot as usize;
            if index < MAX_FILM_GRAIN {
                self.film_grain.available[index] = Some(FgmSlotRecord {
                    chroma_idc: fg.chroma_idc,
                    mlayer_id: obu.header.embedded_layer_id.get(),
                    tlayer_id: obu.header.temporal_layer_id.get(),
                });
            }
        }
    }
}
