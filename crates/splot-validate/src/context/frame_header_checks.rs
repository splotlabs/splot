// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Locally decidable frame-header semantic checks.

use super::*;

/// The cross-OBU HLS-availability state a frame's reference checks consult (AV2 § 7.3.8):
/// per-level quantizer-matrix availability (§ 7.3.8.9 / § 6.17.6.2) and per-slot
/// film-grain availability (§ 7.3.8.8 / § 6.17.10.1). Bundled so the frame-header check
/// keeps one availability parameter as more reference families land.
#[derive(Clone, Copy)]
pub(super) struct FrameReferenceAvailability<'a> {
    /// Per-level custom quantizer-matrix availability (AV2 § 6.17.6.2).
    pub(super) qm: &'a QuantizerMatrixState,
    /// Per-slot film-grain model availability (AV2 § 6.17.10.1 / § 7.3.8.8).
    pub(super) film_grain: &'a FilmGrainState,
    /// The modeled §7.23 per-extended-layer reference-frame buffer view (AV2 § 7.23),
    /// threaded into the core parse so the §6.17.2 inter `ref_frame_idx` validity check
    /// sees the same `RefValid[]` the celu/output decisions do. `unknown()` when the
    /// extended layer has no modeled buffer yet (no false positives).
    pub(super) reference_buffer: FrameReferenceStateView<'a>,
}

/// The frame's linearly-available § 7.3.8.1 random-access-point HLS references, surfaced by
/// [`frame_header_core_checks`] so the caller (in `&mut self` context) can buffer them in the
/// [`RapReplayTracker`]. Each is recorded ONLY when its linear availability check found the
/// object present in-band under external-disabled, keeping the replay predicate disjoint from
/// the linear `frame-header/film-grain-model-unavailable` / `frame-header/qm-level-unavailable`
/// checks.
#[derive(Default)]
pub(super) struct FrameRapReferences {
    /// The referenced film-grain model slot (`fgm_id`), when linearly available.
    pub(super) film_grain_slot: Option<u8>,
    /// The referenced custom quantizer-matrix levels that were linearly available.
    pub(super) qm_levels: Vec<u8>,
}

/// Emits the locally decidable frame-header-info / frame-size diagnostics for a frame
/// whose active sequence header is available (AV2 § 6.17.2 / § 6.17.4 / § 6.4.6).
///
/// Checks decidable from the parsed core and the active sequence alone are emitted here —
/// including the § 6.17.2 `primary_ref_frame < NumTotalRefs` range bound, which needs only
/// the two recorded scalars (`signal_primary_ref_frame` / `primary_ref_frame` and the
/// explicit-map `num_total_refs`) and no reference-frame buffer state. Checks that need the
/// modeled § 7.23 reference-frame buffer (show-existing-frame slot validity, explicit
/// `ref_frame_idx` slot validity) run from the buffer view rather than being guessed; a path
/// whose bound needs unmodeled state (the implicit reference map's `NumTotalRefs`) under-reports.
pub(super) fn frame_header_core_checks(
    obu: &ObuEnvelope<'_>,
    first_picture_in_tu: bool,
    active_sequence: &SequenceHeader,
    mfh_record: Option<&MultiFrameHeaderRecord>,
    reference_state: FrameReferenceAvailability<'_>,
    options: &ValidationOptions,
    report: &mut ValidationReport,
) -> FrameRapReferences {
    const PRIMARY_REF_NONE: u8 = 7;
    let FrameReferenceAvailability {
        qm: qm_state,
        film_grain: film_grain_state,
        reference_buffer,
    } = reference_state;
    if obu.header.obu_type == ObuType::RasFrame
        && let Some(inter) = active_sequence.inter.as_ref()
        && inter.long_term_frame_id_bits == 0
    {
        report.push(frame_header_error(
            "frame-header/ras-requires-long-term-frame-id-bits",
            "6.4.6",
            obu,
            "OBU_RAS_FRAME is present, but the active sequence header has \
             long_term_frame_id_bits == 0"
                .to_owned(),
        ));
    }

    if matches!(obu.header.obu_type, ObuType::Switch | ObuType::RasFrame) {
        let curr = obu.header.embedded_layer_id;
        let map = &active_sequence.general.mlayer_dependency_map;
        for raw_m in 0..MAX_NUM_MLAYERS {
            let m = EmbeddedLayerId::from_bits(raw_m as u8);
            if m == curr {
                continue;
            }
            if map.depends_on(curr, m) {
                report.push(frame_header_error(
                    "frame-header/switch-or-ras-mlayer-dependency-not-self-contained",
                    "6.4.1",
                    obu,
                    format!(
                        "{} with obu_mlayer_id {} has MLayerDependencyMap[{}][{}] != 0 in the \
                         active sequence header, but a SWITCH / RAS frame must not depend on any \
                         other embedded layer",
                        obu.header.obu_type.spec_name(),
                        curr.get(),
                        curr.get(),
                        m.get()
                    ),
                ));
            }
        }
    }

    let max_width = active_sequence.general.max_frame_width.get();
    let max_height = active_sequence.general.max_frame_height.get();

    // AV2 § 6.17.2 (mirror :4348-4349): a `cur_mfh_id > 0` frame's referenced MFH stored dims
    // must satisfy mfh_frame_width/height_minus_1 <= max_frame_width/height_minus_1. Decidable
    // from the resolved MFH record and the sequence maxima alone, so it runs before
    // `parse_frame_core` — a truncated frame header must not silence it. Distinct rule id from
    // the §6.17.4.1 derived-FrameWidth check below (different predicate). An MFH with no
    // `mfh_frame_size` defaults to the sequence maxima (§5.18.2 :4101) and is trivially in
    // range; an unresolvable MFH (`mfh_record == None`) stays silent.
    let mfh_stored_dims = if let Some(record) = mfh_record
        && let Some(mfh_size) = record.mfh_frame_size
    {
        let mfh_width = mfh_size.width_minus_1 + 1;
        let mfh_height = mfh_size.height_minus_1 + 1;
        if mfh_width > max_width || mfh_height > max_height {
            report.push(frame_header_error(
                "frame-header/mfh-frame-size-exceeds-sequence-max",
                "6.17.2",
                obu,
                format!(
                    "the referenced multi-frame header (cur_mfh_id {}) stores \
                     FrameWidth={}, FrameHeight={}, which exceeds the active sequence \
                     maximum {}x{} (§6.17.2 mfh_frame_width/height_minus_1 <= \
                     max_frame_width/height_minus_1)",
                    record.mfh_id.get(),
                    mfh_width,
                    mfh_height,
                    max_width,
                    max_height
                ),
            ));
        }
        Some((mfh_width, mfh_height))
    } else {
        None
    };

    let Some(core) = parse_frame_core(
        obu,
        first_picture_in_tu,
        active_sequence,
        mfh_record,
        reference_buffer,
    ) else {
        return FrameRapReferences::default();
    };

    if core.status.is_truncated_in_modeled_region() {
        report.push(frame_header_error(
            "frame-header/truncated-frame-header",
            "6.2.1",
            obu,
            format!(
                "the OBU payload ends inside the frame header before mandatory \
                 frame_header_info() syntax (§5.18.2) could be read (parse stopped: {}); \
                 the §6.2.1 OBU payload must contain every mandatory frame-header syntax \
                 element",
                core.status.label()
            ),
        ));
    }

    if let Some(violation) = core.sef_trailing_bits
        && let Some(message) = violation.violation_message()
    {
        report.push(frame_header_error(
            "frame-header/sef-trailing-bits-invalid",
            "6.2.3",
            obu,
            format!(
                "the show-existing-frame OBU payload's §5.2.3 trailing_bits() is malformed: \
                 {message}"
            ),
        ));
    }

    if let Some(idx) = core.bridge_frame_ref_idx
        && let Some(inter) = active_sequence.inter.as_ref()
        && idx >= u32::from(inter.num_ref_frames)
    {
        report.push(frame_header_error(
            "frame-header/bridge-ref-index-out-of-range",
            "6.17.2",
            obu,
            format!(
                "bridge_frame_ref_idx {idx} must be less than NumRefFrames {}",
                inter.num_ref_frames
            ),
        ));
    }

    if let Some(inter) = core.inter.as_ref()
        && let Some(num_total_refs) = inter.num_total_refs
        && let Some(seq_inter) = active_sequence.inter.as_ref()
    {
        const REFS_PER_FRAME: u32 = 7;
        let active_num_ref_frames = REFS_PER_FRAME.min(u32::from(seq_inter.num_ref_frames));
        if num_total_refs > active_num_ref_frames {
            report.push(frame_header_error(
                "frame-header/num-total-refs-out-of-range",
                "6.17.2",
                obu,
                format!(
                    "num_total_refs {num_total_refs} must be less than or equal to \
                     ActiveNumRefFrames {active_num_ref_frames} = Min(REFS_PER_FRAME {}, \
                     NumRefFrames {})",
                    REFS_PER_FRAME, seq_inter.num_ref_frames
                ),
            ));
        }
    }

    if let Some(inter) = core.inter.as_ref()
        && inter.signal_primary_ref_frame == Some(true)
        && let Some(primary_ref_frame) = inter.primary_ref_frame
        && let Some(num_total_refs) = inter.num_total_refs
        && primary_ref_frame != PRIMARY_REF_NONE
        && u32::from(primary_ref_frame) >= num_total_refs
    {
        report.push(frame_header_error(
            "frame-header/primary-ref-frame-out-of-range",
            "6.17.2",
            obu,
            format!(
                "primary_ref_frame {primary_ref_frame} is present in the bitstream \
                 (signal_primary_ref_frame == 1) but is neither PRIMARY_REF_NONE ({PRIMARY_REF_NONE}) \
                 nor less than NumTotalRefs ({num_total_refs}), violating the §6.17.2 requirement \
                 of bitstream conformance"
            ),
        ));
    }

    if let Some(inter) = core.inter.as_ref()
        && inter.use_bru == Some(true)
    {
        if let Some(bru_ref) = inter.bru_ref
            && let Some(num_total_refs) = inter.num_total_refs
            && bru_ref >= num_total_refs
        {
            report.push(frame_header_error(
                "frame-header/bru-ref-out-of-range",
                "6.17.2",
                obu,
                format!(
                    "bru_ref {bru_ref} must be less than NumTotalRefs ({num_total_refs}) \
                     when use_bru == 1"
                ),
            ));
        }
        if core.immediate_output_frame == Some(false) {
            report.push(frame_header_error(
                "frame-header/bru-without-immediate-output",
                "6.17.2",
                obu,
                "use_bru == 1 requires immediate_output_frame == 1".to_string(),
            ));
        }
    }

    if core
        .inter
        .as_ref()
        .is_some_and(|inter| inter.has_invalid_ref_frame_idx)
    {
        report.push(frame_header_error(
            "frame-header/ref-frame-idx-invalid-slot",
            "6.17.2",
            obu,
            "a ref_frame_idx[i] names a reference slot the modeled §7.23 reference state \
             proves invalid (RefValid[idx] == 0) or out of the NUM_REF_FRAMES buffer"
                .to_owned(),
        ));
    }

    // AV2 § 6.17.4.1 (mirror :5200-5205): derived FrameWidth/Height must not exceed the
    // sequence maximum. The `cur_mfh_id > 0` non-override path carries the MFH's stored dims
    // verbatim (mirror :5767), already covered by the §6.17.2 check above, so the derived
    // check defers on that parsed path to avoid double-reporting. The suppression keys on
    // provenance (the override flag), not on dimension equality: an override==1 frame coding
    // the same out-of-range dims commits a separate §6.17.4.1 violation through its own fields.
    let derived_is_mfh_default = core.frame_size_override_flag == Some(false)
        && !core.cur_mfh_id.is_zero()
        && mfh_stored_dims.is_some();
    if !derived_is_mfh_default
        && let Some(size) = core.frame_size
        && (size.width > max_width || size.height > max_height)
    {
        report.push(frame_header_error(
            "frame-header/frame-size-exceeds-sequence-max",
            "6.17.4.1",
            obu,
            format!(
                "frame_header_info() derives FrameWidth={}, FrameHeight={}, which \
                 exceeds the active sequence maximum {}x{}",
                size.width, size.height, max_width, max_height
            ),
        ));
    }

    if core.forbidden_ref_long_term_id {
        report.push(frame_header_error(
            "frame-header/ref-long-term-id-reserved",
            "6.17.2",
            obu,
            "a ref_long_term_id[i] equals the reserved (1 << long_term_frame_id_bits) - 1"
                .to_owned(),
        ));
    }

    if core.immediate_output_frame == Some(false) && core.refresh_frame_flags == Some(0) {
        report.push(frame_header_error(
            "frame-header/refresh-frame-flags-zero-on-deferred-output",
            "6.17.2",
            obu,
            "immediate_output_frame == 0 requires refresh_frame_flags to be nonzero".to_owned(),
        ));
    }

    if active_sequence.general.still_picture
        && (matches!(core.frame_type, Some(frame_type) if frame_type != FrameType::Key)
            || core.immediate_output_frame == Some(false))
    {
        report.push(frame_header_error(
            "frame-header/still-picture-requires-key-frame",
            "6.17.2",
            obu,
            "a still_picture sequence requires a KEY_FRAME with immediate_output_frame == 1"
                .to_owned(),
        ));
    }

    if let Some(tile_info) = core.tile_info.as_ref() {
        frame_tile_info_checks(tile_info, obu, report);
    }

    tile_group_range_checks(
        obu,
        first_picture_in_tu,
        active_sequence,
        mfh_record,
        report,
    );

    frame_annex_a_level_checks(&core, active_sequence, obu, report);

    let qm_rap_levels = if let Some(setup_qm) = core.setup_qm_params.as_ref() {
        frame_qm_reference_checks(setup_qm, active_sequence, qm_state, options, obu, report)
    } else {
        Vec::new()
    };

    if let Some(ccso) = core.ccso_params.as_ref() {
        frame_ccso_params_checks(ccso, obu, report);
    }

    let film_grain_rap_slot = if let Some(film_grain) = core
        .sef_film_grain
        .as_ref()
        .or_else(|| core.intra_tail.as_ref().map(|tail| &tail.film_grain))
    {
        frame_film_grain_reference_checks(
            *film_grain,
            film_grain_state,
            active_sequence,
            options,
            obu,
            report,
        )
    } else {
        None
    };

    let rap_refs = FrameRapReferences {
        film_grain_slot: film_grain_rap_slot,
        qm_levels: qm_rap_levels,
    };

    let Some(num_ref_frames) = active_sequence
        .inter
        .as_ref()
        .map(|inter| u32::from(inter.num_ref_frames))
    else {
        return rap_refs;
    };
    let Some(refresh) = core.refresh_frame_flags else {
        return rap_refs;
    };
    let Some(all_slots_plus_1) = 1u32.checked_shl(num_ref_frames) else {
        return rap_refs;
    };

    if refresh >= all_slots_plus_1 {
        report.push(frame_header_error(
            "frame-header/frame-to-refresh-out-of-range",
            "6.17.2",
            obu,
            format!(
                "refresh_frame_flags {refresh:#x} sets a reference slot at or beyond \
                 NumRefFrames {num_ref_frames} (frame_to_refresh must be less than NumRefFrames)"
            ),
        ));
    }

    if core.frame_type == Some(FrameType::IntraOnly)
        && num_ref_frames > 1
        && refresh == all_slots_plus_1 - 1
    {
        report.push(frame_header_error(
            "frame-header/intra-only-refresh-all-slots",
            "6.17.2",
            obu,
            format!(
                "an INTRA_ONLY_FRAME with NumRefFrames {num_ref_frames} must not set \
                 refresh_frame_flags to all slots"
            ),
        ));
    }

    rap_refs
}
