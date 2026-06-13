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
) -> Option<u8> {
    let FrameReferenceAvailability {
        qm: qm_state,
        film_grain: film_grain_state,
        reference_buffer,
    } = reference_state;
    // AV2 § 6.4.6: if long_term_frame_id_bits == 0, no OBU_RAS_FRAME shall be present
    // in the coded video sequence. Decidable from obu_type + the active sequence alone.
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

    // AV2 § 6.4.1: "If obu_type is equal to either OBU_SWITCH or OBU_RAS_FRAME, it is a
    // requirement of bitstream conformance that, for any embedded layer ID m not equal
    // to obu_mlayer_id, MLayerDependencyMap[obu_mlayer_id][m] shall be equal to 0."
    // (docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1, lines 615-617). A
    // SWITCH / RAS frame must be self-contained: its embedded layer may not depend on
    // any other embedded layer. Decidable from obu_type + obu_mlayer_id + the active
    // sequence's MLayerDependencyMap alone, like the § 6.4.6 RAS check above.
    if matches!(obu.header.obu_type, ObuType::Switch | ObuType::RasFrame) {
        let curr = obu.header.embedded_layer_id;
        let map = &active_sequence.general.mlayer_dependency_map;
        // Scanning the full 0..MAX_NUM_MLAYERS range never reports a layer undeclared by
        // this sequence header: the § 5.4.1 parser only ever sets MLayerDependencyMap
        // entries where refLayer <= currLayer <= max_mlayer_id (default fill and signaled
        // override alike), so depends_on(curr, m) is unconditionally false for any
        // m > max_mlayer_id and the dependency-scope constraint cannot yield a false
        // positive here.
        for raw_m in 0..MAX_NUM_MLAYERS {
            // raw_m fits in the 3-bit obu_mlayer_id range (MAX_NUM_MLAYERS == 8).
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

    // AV2 § 6.17.2: after load_sequence_header(), for every `cur_mfh_id > 0` frame it is a
    // requirement of bitstream conformance that the *referenced multi-frame header's stored*
    // dimensions satisfy mfh_frame_width_minus_1[ cur_mfh_id ] <= max_frame_width_minus_1 and
    // mfh_frame_height_minus_1[ cur_mfh_id ] <= max_frame_height_minus_1
    // (docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2, mirror :4348-4349).
    // This bounds the MFH's *stored* dims and is INDEPENDENT of frame_size_override_flag and
    // of how far the referencing frame header parses: it is decidable from the resolved MFH
    // record and the active sequence maxima alone, at the load_sequence_header point. So it
    // runs here, BEFORE (and independent of) the `parse_frame_core` outcome below — a
    // truncated / malformed frame-header remainder (`core == None`) must not silence this
    // decidable diagnostic. A frame overriding to in-range dims (so `core.frame_size` is
    // conformant) still must not reference an out-of-range MFH. The predicate (stored MFH
    // dims) differs from the §6.17.4.1 derived-FrameWidth check below, so it has its own
    // rule id. An MFH with no `mfh_frame_size` payload infers its default dims to the
    // sequence maxima (§5.18.2, mirror :4101) and is trivially in range, so the omitted-size
    // case is silent here. Anchored at `obu` (the referencing frame's OBU) and emitted once
    // per referencing frame header. On this resolution path `record.mfh_id == cur_mfh_id`
    // (`resolve_frame_mfh_record` looks the record up by the prefix's `cur_mfh_id`), so the
    // message's id matches the referencing frame's `cur_mfh_id`. An unresolvable MFH leaves
    // `mfh_record == None` (the shared guard) and stays silent.
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

    // This call site emits the §6.17 bridge-ref / frame-size / tile / quant diagnostics.
    // A `cur_mfh_id > 0` frame's FrameWidth/FrameHeight come from `mfh_record` on the
    // non-override path (§5.18.4.1, mirror :5767), so the resolved record is threaded in
    // with the §7.3.8.7 discipline (the caller passes `resolve_frame_mfh_record`'s result).
    // For a `cur_mfh_id == 0` frame, or a `cur_mfh_id > 0` frame whose in-band MFH is
    // unresolvable, this is `None` and the core parser keeps its existing early-stop. The
    // §6.17.2 stored-MFH bound above already ran, so it is not lost when the core parse stops.
    //
    // The §7.23 reference-frame buffer view is threaded in by the caller (the modeled
    // per-extended-layer buffer, or `unknown()` when none is established): the §6.17.2
    // inter `ref_frame_idx` validity check below consults the same `RefValid[]` the
    // celu/output decisions do. The other §6.17 diagnostics this function emits are
    // decidable from the active sequence header alone and ignore the view.
    let core = parse_frame_core(
        obu,
        first_picture_in_tu,
        active_sequence,
        mfh_record,
        reference_buffer,
    )?;

    // AV2 § 6.2.1 / § 5.18.2: the frame_header_info() syntax elements (§ 5.18.2) are
    // mandatory — `frame_header( )` reads them sequentially from the OBU payload inside
    // open_bitstream_unit() (§ 5.2.1). The payload is bounded by obuPayloadSize and "lies
    // between the first bit of the given bytes and the last bit before the first trailing
    // bit" (§ 6.2.1, docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-1, lines
    // 47-60); trailing bits are always present (unless header-only), so a payload that ends
    // BEFORE a mandatory syntax element is malformed — the §6.2.1 NOTE makes "the parsing
    // of the OBU header and payload leads to the consumption of bits within the trailing
    // bits" a detectable error condition. The core parser preserves the already-parsed
    // facts and reports the truncation through one of the EOF-in-a-fully-modeled-region
    // statuses (StoppedInsideFilterParams / StoppedInsideIntraTail /
    // StoppedInsideShowExistingFrame). Those — and ONLY those — are a decidable defect:
    // an unsupported-coverage stop (StoppedBeforeWienerNsFilter, UnsupportedUntilFeature,
    // the MFH-unresolvable stops, CoreFieldsOnly) stops where this parser does not fully
    // model the following syntax, so its early end is not evidence of truncation and must
    // stay silent. `is_truncated_in_modeled_region()` is the exact partition (documented on
    // FrameHeaderParseStatus). Anchored at the frame's OBU. The facts path is untouched:
    // the preserved core fields still feed every diagnostic below, so a truncated frame
    // keeps contributing its decided facts (celu / frame-unit judgments unchanged).
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

    // AV2 § 5.2.1 (:124-152) / § 5.2.3 / § 6.2.1: a show-existing-frame OBU's payload is
    // exactly the SEF frame_header() plus trailing_bits( remainingPayloadBits ). The SEF
    // arm of § 5.18.2 (mirror :4145) return()s right after film_grain_config() (:4186), and
    // a SEF OBU is not an is_tile_group() type, so usedArith == 0 and there is no tile data
    // — the boundary is decidable from the payload alone. A non-conformant tail (no
    // trailing_one_bit, or a stray set bit after it, including the grain_seed-eats-the-marker
    // case) is a § 6.2.1 / § 6.2.3 conformance defect. The core parser classifies the tail
    // without failing (the parsed SEF facts survive), so surface a non-Valid outcome here.
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

    // AV2 § 6.17.2: bridge_frame_ref_idx must name a valid reference slot, so it must
    // be less than NumRefFrames.
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

    // AV2 § 6.17.2 (mirror :4578-4579): if num_total_refs is present, it is a requirement
    // of bitstream conformance that num_total_refs <= ActiveNumRefFrames, where
    // ActiveNumRefFrames = Min( REFS_PER_FRAME, NumRefFrames ) (§ 5.18.2 mirror :963;
    // REFS_PER_FRAME == 7 per § 3 mirror :697). num_total_refs is read as f(3) so the parse
    // is always safe (the ref_frame_idx loop runs <= 7 times); this is a decidable
    // bound the validator checks from the recorded value and the active sequence alone.
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

    // AV2 § 6.17.2 (mirror :4500-4502): "when primary_ref_frame is present in the bitstream
    // primary_ref_frame is either equal to PRIMARY_REF_NONE, or primary_ref_frame is less
    // than NumTotalRefs"
    // (docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2). "Present in the
    // bitstream" is exactly the signaled case: primary_ref_frame is read as f(3) only when
    // signal_primary_ref_frame == 1 (§5.18.2 mirror :4391-4399); when it is inferred
    // (PRIMARY_REF_NONE on the switch / bridge / intra arms, or PRIMARY_REF_CHOOSE when
    // signal == 0) the constraint is satisfied trivially and no range check applies. The
    // bound needs NumTotalRefs, which only the explicit-reference-map arm records; the
    // implicit `get_ref_frames()` map (unmodeled) records `num_total_refs == None`, so this
    // check under-reports there (stays silent rather than guessing NumTotalRefs). Decidable
    // from the two recorded scalars alone — no reference-frame buffer state is required.
    const PRIMARY_REF_NONE: u8 = 7;
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

    // AV2 § 6.17.2 (mirror :4587-4596): "If use_bru is equal to 1, it is a requirement of
    // bitstream conformance that all the following are true: … immediate_output_frame is
    // equal to 1, bru_ref is less than NumTotalRefs, …". The two pure-arithmetic clauses
    // are decidable from the recorded scalars alone; the slot-fact clauses (RefOrderHint /
    // RESTRICTED_OH, the dims equalities) and the refresh-bit clause need reference-state /
    // map facts the explicit-map arm may not ground and stay named residuals. The implicit
    // `get_ref_frames()` map records `num_total_refs == None`, so the bru_ref bound
    // under-reports there (silent rather than guessing NumTotalRefs).
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

    // AV2 § 6.17.2: every used reference slot must be valid — an inter frame's
    // ref_frame_idx[i] must name a slot whose RefValid is 1. The core parser flags a
    // parsed ref_frame_idx that the modeled §7.23 reference state proves invalid
    // (RefValid[idx] == 0 against a modeled buffer, or an out-of-NUM_REF_FRAMES index).
    // Slots the model has not grounded stay Unknown and are not reported (no guessing).
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

    // FrameWidth/FrameHeight do not exceed the active sequence maximum
    // (FrameWidth <= MaxFrameWidth, FrameHeight <= MaxFrameHeight). On the explicit
    // override path this is AV2 § 6.17.4.1 (frame_width_minus_1 <= max_frame_width_minus_1,
    // frame_height_minus_1 <= max_frame_height_minus_1,
    // docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1, mirror :5200-5205).
    // On the `cur_mfh_id > 0` non-override path FrameWidth = mfh_frame_width_minus_1 + 1
    // (mirror :5767), so `core.frame_size` carries the MFH's stored dims verbatim — that
    // exact case is already the §6.17.2 stored-MFH check above, the single home for
    // stored-MFH dims. To avoid double-reporting the identical numbers, the derived check
    // defers ONLY on that parsed PATH — `frame_size_override_flag == 0` on a resolved
    // `cur_mfh_id > 0` frame (§5.18.4 / §5.18.2, mirror :5767), where FrameWidth/Height are
    // the MFH default dimensions and carry no explicit fields of their own. The suppression
    // keys on provenance (the override flag), NOT on dimension equality: an override==1 frame
    // that explicitly codes the same out-of-range dims the MFH stores commits a genuine,
    // separate §6.17.4.1 violation through its own frame_width/height_minus_1 fields, so both
    // checks legitimately fire even when the numbers coincide. (`mfh_stored_dims.is_some()`
    // bounds the deferral to the case the §6.17.2 home actually examined those dims.)
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

    // AV2 § 6.17.2: ref_long_term_id[i] != (1 << long_term_frame_id_bits) - 1.
    if core.forbidden_ref_long_term_id {
        report.push(frame_header_error(
            "frame-header/ref-long-term-id-reserved",
            "6.17.2",
            obu,
            "a ref_long_term_id[i] equals the reserved (1 << long_term_frame_id_bits) - 1"
                .to_owned(),
        ));
    }

    // AV2 § 6.17.2: if immediate_output_frame == 0, refresh_frame_flags must be nonzero
    // (a deferred-output frame must update at least one reference slot).
    if core.immediate_output_frame == Some(false) && core.refresh_frame_flags == Some(0) {
        report.push(frame_header_error(
            "frame-header/refresh-frame-flags-zero-on-deferred-output",
            "6.17.2",
            obu,
            "immediate_output_frame == 0 requires refresh_frame_flags to be nonzero".to_owned(),
        ));
    }

    // AV2 § 6.17.2: still_picture == 1 requires FrameType == KEY_FRAME and
    // immediate_output_frame == 1.
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

    // AV2 § 6.17.7.2: tile-info bounds for a parsed `tile_info()`.
    if let Some(tile_info) = core.tile_info.as_ref() {
        frame_tile_info_checks(tile_info, obu, report);
    }

    // AV2 § 5.19 / § 6.18: the post-frame-header tile_group_obu() structure — the
    // tile-group range (tg_start/tg_end) and the headerBytes/payload boundary — is
    // decidable on the intra-complete first-tile-group path (use_bru/bru_inactive derive
    // to 0). Emits the locally-decidable §6.18 tg-range diagnostics; a non-intra-complete
    // or non-first-tile-group frame is the BRU-undecidable honest stop and stays silent.
    tile_group_range_checks(
        obu,
        first_picture_in_tu,
        active_sequence,
        mfh_record,
        report,
    );

    // Annex A.4 static level limits for the parsed frame size / tile count against the
    // active sequence header's seq_level_idx / seq_tier.
    frame_annex_a_level_checks(&core, active_sequence, obu, report);

    // AV2 § 6.17.6.2 / § 7.3.8.9: custom-QM plane-count references and the §7.3.8.9
    // availability presence for a parsed `setup_qm_params()`, gated on recorded
    // quantizer-matrix availability state.
    if let Some(setup_qm) = core.setup_qm_params.as_ref() {
        frame_qm_reference_checks(setup_qm, active_sequence, qm_state, options, obu, report);
    }

    // AV2 § 6.17.7.8: per-plane CCSO field bounds for a parsed `ccso_params()`.
    if let Some(ccso) = core.ccso_params.as_ref() {
        frame_ccso_params_checks(ccso, obu, report);
    }

    // AV2 § 6.17.10.1 / § 7.3.8.8: when `apply_grain == 1`, a film grain OBU that has set
    // FilmGrainPresent[ fgm_id ] == 1 for the referenced fgm_id must be available, and (when
    // an in-band model is recorded) the three § 6.17.10.1 layer-dependency / chroma
    // constraints must hold against the active sequence header's § 5.4.1 maps. The parsed
    // film_grain_config() lives on the SEF path (`sef_film_grain`) or the intra tail
    // (`intra_tail.film_grain`).
    let film_grain_rap_slot = if let Some(film_grain) = core
        .sef_film_grain
        .as_ref()
        .or_else(|| core.intra_tail.as_ref().map(|tail| &tail.film_grain))
    {
        frame_film_grain_reference_checks(
            film_grain,
            film_grain_state,
            active_sequence,
            options,
            obu,
            report,
        )
    } else {
        None
    };

    // The remaining checks compare refresh_frame_flags against NumRefFrames.
    let Some(num_ref_frames) = active_sequence
        .inter
        .as_ref()
        .map(|inter| u32::from(inter.num_ref_frames))
    else {
        return film_grain_rap_slot;
    };
    let Some(refresh) = core.refresh_frame_flags else {
        return film_grain_rap_slot;
    };
    // 1 << NumRefFrames as the exclusive upper bound of a valid refresh mask.
    let Some(all_slots_plus_1) = 1u32.checked_shl(num_ref_frames) else {
        return film_grain_rap_slot;
    };

    // AV2 § 6.17.2: frame_to_refresh < NumRefFrames. In the compact refresh mode
    // refresh_frame_flags == 1 << frame_to_refresh, so an out-of-range slot is exactly a
    // mask with a bit at or beyond NumRefFrames; the full and all-frames forms are always
    // below 1 << NumRefFrames.
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

    // AV2 § 6.17.2: an INTRA_ONLY_FRAME with NumRefFrames > 1 must not refresh every slot
    // (refresh_frame_flags != (1 << NumRefFrames) - 1).
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

    film_grain_rap_slot
}
