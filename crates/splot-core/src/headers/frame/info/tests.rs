// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unit tests for the [`super`] AV2 § 5.18 frame-header parser.

use super::*;
use crate::error::Error;
use crate::headers::frame::filtering::InterpolationFilter;
use crate::headers::frame::inter::{InterStop, MvPrecision, NUM_REF_FRAMES};
use crate::headers::frame::restoration::FrameRestorationType;
use crate::headers::frame::tail::TxMode;
use crate::segment::{MAX_SEGMENTS, SEG_LVL_MAX, SegmentFeature, SegmentInfo};
use crate::span::ByteOffset;

use crate::test_bits::Bits;

fn base_seq() -> CoreSeqView {
    CoreSeqView::new_minimal_intra(4096, 2304).expect("4096x2304 is a valid maximum")
}

/// Parses the activation prefix then the core body, returning the result and the
/// total bits consumed (prefix + body). `cur_mfh_id == 0` paths pass no MFH state.
fn parse_body(
    data: &[u8],
    obu_type: ObuType,
    first_picture_in_tu: bool,
    seq: &CoreSeqView,
) -> Result<(FrameHeaderCore, u64)> {
    parse_body_with_mfh(data, obu_type, first_picture_in_tu, seq, None)
}

/// Like [`parse_body`] but resolves a `cur_mfh_id > 0` reference against `mfh_view`
/// (the in-band multi-frame-header state) when present.
fn parse_body_with_mfh(
    data: &[u8],
    obu_type: ObuType,
    first_picture_in_tu: bool,
    seq: &CoreSeqView,
    mfh_view: Option<&MfhFrameView>,
) -> Result<(FrameHeaderCore, u64)> {
    parse_body_with_ref(
        data,
        obu_type,
        first_picture_in_tu,
        seq,
        mfh_view,
        &FrameReferenceStateView::unknown(),
    )
}

/// Like [`parse_body_with_mfh`] but threads a modeled reference state into the core
/// body (the inter reference paths consume it).
fn parse_body_with_ref(
    data: &[u8],
    obu_type: ObuType,
    first_picture_in_tu: bool,
    seq: &CoreSeqView,
    mfh_view: Option<&MfhFrameView>,
    reference_state: &FrameReferenceStateView<'_>,
) -> Result<(FrameHeaderCore, u64)> {
    let mut reader = BitReader::new(data, ByteOffset::new(0));
    let prefix = parse_frame_header_prefix(&mut reader, obu_type, Some(first_picture_in_tu))?;
    let mut core = init_core_from_prefix(&prefix, obu_type, first_picture_in_tu);
    parse_core_body(&mut reader, &mut core, seq, mfh_view, reference_state)?;
    let consumed = reader.consumed_bits();
    Ok((core, consumed))
}

#[test]
fn frame_header_core_reads_direct_sequence_reference() {
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(1); // seq_header_id_in_frame_header
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(1); // frame_size_override_flag
    bits.f(5, 4); // order_hint
    bits.f(1920 - 1, 12); // frame_width_minus_1
    bits.f(1080 - 1, 12); // frame_height_minus_1
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    bits.bit(1); // uniform_tile_spacing_flag
    bits.bit(0); // increment_tile_cols_log2 = 0
    bits.bit(0); // increment_tile_rows_log2 = 0
    bits.f(90, 8); // base_q_idx
    bits.bit(0); // segmentation_enabled (§ 5.18.7.1)
    bits.bit(0); // using_qmatrix (§ 5.18.6.2)
    bits.bit(0); // delta_q_present (§ 5.18.7.8, base_q_idx > 0)
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1] (both 0 -> no chroma pair, no delta-Q)
    bits.bit(0); // tx_mode_select = 0 -> TX_MODE_LARGEST
    bits.f(0, 2); // reduced_tx_set = 0
    let data = bits.into_bytes();
    let (core, consumed) = parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap();

    assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
    assert!(core.cur_mfh_id.is_zero());
    assert_eq!(core.seq_header_id_in_frame_header, Some(1));
    assert_eq!(core.frame_type, Some(FrameType::Key));
    assert_eq!(core.frame_is_intra, Some(true));
    assert_eq!(core.immediate_output_frame, Some(false));
    assert_eq!(core.implicit_output_frame, Some(false));
    assert_eq!(core.order_hint_lsb, Some(5));
    assert_eq!(core.refresh_frame_flags, Some((1 << 8) - 1));
    assert_eq!(core.frame_size, Some(FrameSize::new(1920, 1080)));
    assert_eq!(core.allow_screen_content_tools, Some(false));
    assert_eq!(core.allow_intrabc, Some(false));
    assert_eq!(core.disable_cdf_update, Some(false));
    let tile_info = core.tile_info.as_ref().unwrap();
    assert_eq!(tile_info.tile_cols, 1);
    assert_eq!(tile_info.tile_rows, 1);
    assert_eq!(tile_info.context_update_tile_id, 0);
    assert_eq!(tile_info.tile_size_bytes, None);
    assert_eq!(core.quantization_params.unwrap().base_q_idx, 90);
    assert!(!core.segmentation_params.unwrap().segmentation_enabled);
    assert!(!core.setup_qm_params.unwrap().using_qmatrix);
    assert!(!core.delta_q_params.unwrap().delta_q_present);
    let lossless = core.lossless_info.unwrap();
    assert!(!lossless.coded_lossless);
    assert!(!lossless.has_lossless_segment);
    assert!(!lossless.allow_tcq);
    assert!(!lossless.allow_parity_hiding);
    let deblocking = core.deblocking_filter_params.unwrap();
    assert_eq!(deblocking.apply_deblocking_filter, [false; 4]);
    assert!(!core.gdf_params.unwrap().gdf_frame_enable);
    assert!(!core.cdef_params.unwrap().cdef_frame_enable);
    let tail = core.intra_tail.as_ref().unwrap();
    assert_eq!(tail.tx_mode, TxMode::Largest);
    assert_eq!(tail.reduced_tx_set, 0);
    assert!(!tail.film_grain.apply_grain);
    assert_eq!(consumed, 4 + 33 + 14 + 2 + 3);
}

/// A fixed in-band multi-frame-header record resolving `cur_mfh_id` for the
/// `cur_mfh_id > 0` core path. `mfh_frame_size` / `mfh_seg_info_present_flag`
/// control which § 5.18.4.1 / § 5.18.7.1 arm is exercised.
fn mfh_record(
    mfh_frame_size: Option<crate::hls::MfhFrameSize>,
    seg: Option<&(bool, bool, SegmentInfo)>,
) -> MultiFrameHeaderRecord {
    let (present, ext, allow, info) = match seg {
        Some(&(ext, allow, info)) => (true, Some(ext), Some(allow), Some(info)),
        None => (false, None, None, None),
    };
    MultiFrameHeaderRecord {
        mfh_id: MfhId::from_raw(1),
        mfh_seq_header_id: SequenceHeaderId::try_new(0).unwrap(),
        mfh_tlayer_id: crate::types::TemporalLayerId::from_bits(0),
        mfh_mlayer_id: crate::types::EmbeddedLayerId::from_bits(0),
        mfh_frame_size,
        mfh_seg_info_present_flag: present,
        mfh_ext_seg_flag: ext,
        mfh_allow_seg_info_change: allow,
        mfh_segment_info: info,
        mfh_deblocking_filter_update: false,
        mfh_apply_deblocking_filter: [false; 4],
        offset: ByteOffset::new(0),
    }
}

/// Like [`mfh_record`] but sets the § 5.18.5.2 deblocking-update arm inputs.
fn mfh_record_with_deblocking(
    mfh_frame_size: Option<crate::hls::MfhFrameSize>,
    update: bool,
    apply: [bool; 4],
) -> MultiFrameHeaderRecord {
    let mut record = mfh_record(mfh_frame_size, None);
    record.mfh_deblocking_filter_update = update;
    record.mfh_apply_deblocking_filter = apply;
    record
}

#[test]
fn frame_header_core_mfh_deblocking_update_copies_apply_no_apply_bits() {
    let mfh_size = Some(crate::hls::MfhFrameSize {
        width_bits: 12,
        height_bits: 12,
        width_minus_1: 1920 - 1,
        height_minus_1: 1080 - 1,
    });
    let record = mfh_record_with_deblocking(mfh_size, true, [true, false, true, true]);
    let view = MfhFrameView::from_record(&record, &base_seq());

    let mut bits = Bits::default();
    bits.uvlc(1); // cur_mfh_id == 1
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(0); // frame_size_override_flag == 0 (MFH default dims, no bits)
    bits.f(7, 4); // order_hint
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    bits.bit(1); // uniform_tile_spacing_flag (single tile)
    bits.bit(0); // increment_tile_cols_log2
    bits.bit(0); // increment_tile_rows_log2
    bits.f(70, 8); // base_q_idx (non-lossless)
    bits.bit(0); // segmentation_enabled
    bits.bit(0); // using_qmatrix
    bits.bit(0); // delta_q_present
    bits.bit(0); // df_delta_q_present[0]
    bits.bit(0); // df_delta_q_present[2]
    bits.bit(0); // df_delta_q_present[3]
    bits.bit(0); // tx_mode_select = 0
    bits.f(0, 2); // reduced_tx_set = 0
    let data = bits.into_bytes();
    let (core, _) = parse_body_with_mfh(
        &data,
        ObuType::ClosedLoopKey,
        true,
        &base_seq(),
        Some(&view),
    )
    .unwrap();

    let deblocking = core.deblocking_filter_params.unwrap();
    assert_eq!(
        deblocking.apply_deblocking_filter,
        [true, false, true, true],
        "the MFH update arm copies apply_deblocking_filter from the record"
    );
    assert_eq!(deblocking.df_delta_q, [0; 4]);
    assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
    assert!(core.intra_tail.is_some());
}

/// A `base_seq()` whose `order_hint_bits` is widened to 5 so the intra body built by
/// [`intra_body_up_to_filter_cluster`] ends on a byte boundary, putting the start of
/// the loop-filter cluster exactly at bit 48 (byte 6). This lets the truncation tests
/// land an EOF at a precise byte without disturbing the preceding structures.
fn byte_aligned_filter_seq() -> CoreSeqView {
    let mut seq = base_seq();
    seq.order_hint_bits = 5;
    seq
}

/// Builds an intra CLK frame-header body parsed cleanly through the § 5.18.2 structure
/// cluster (frame_size 16x16, both output flags 0) up to and including
/// `delta_q_present`, i.e. positioned exactly at the start of the loop-filter cluster.
/// The caller appends the loop-filter bits and applies the truncation. Paired with
/// [`byte_aligned_filter_seq`] (`order_hint_bits == 5`) the cluster starts at bit 48.
fn intra_body_up_to_filter_cluster() -> Bits {
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(0); // immediate_output_frame == 0
    bits.bit(0); // implicit_output_frame == 0
    bits.bit(1); // frame_size_override_flag
    bits.f(5, 5); // order_hint f(order_hint_bits == 5)
    bits.f(16 - 1, 12); // frame_width_minus_1 -> FrameWidth 16
    bits.f(16 - 1, 12); // frame_height_minus_1 -> FrameHeight 16
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    bits.bit(1); // uniform_tile_spacing_flag (single tile)
    bits.f(90, 8); // base_q_idx (non-lossless -> deblocking reads apply bits)
    bits.bit(0); // segmentation_enabled
    bits.bit(0); // using_qmatrix
    bits.bit(0); // delta_q_present
    bits
}

/// Asserts the control-region facts parsed before the loop-filter cluster survived a
/// mid-cluster truncation: the parse returned Ok, the frame size and output flags are
/// intact, and the status records the truncation.
fn assert_truncated_filter_cluster_preserves_facts(core: &FrameHeaderCore) {
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::StoppedInsideFilterParams,
        "a mid-cluster truncation reports StoppedInsideFilterParams"
    );
    assert_eq!(
        core.frame_size,
        Some(FrameSize::new(16, 16)),
        "frame_size parsed before the cluster must survive the truncation"
    );
    assert_eq!(core.immediate_output_frame, Some(false));
    assert_eq!(core.implicit_output_frame, Some(false));
    assert!(
        core.quantization_params.is_some(),
        "quantization_params parsed before the cluster must survive"
    );
    assert!(
        core.tile_info.is_some(),
        "tile_info parsed before the cluster must survive"
    );
}

#[test]
fn frame_header_core_eof_inside_deblocking_filter_params_preserves_facts() {
    let mut bits = intra_body_up_to_filter_cluster();
    let cluster_start = bits.bit_len();
    assert_eq!(
        cluster_start, 48,
        "with order_hint_bits == 5 the loop-filter cluster starts on byte 6"
    );
    bits.bit(0); // apply_deblocking_filter[0] (in the dropped byte 6)
    let mut data = bits.into_bytes();
    data.truncate(6); // 48 bits: the deblocking apply reads overrun
    let (core, _) = parse_body(
        &data,
        ObuType::ClosedLoopKey,
        true,
        &byte_aligned_filter_seq(),
    )
    .unwrap();
    assert_truncated_filter_cluster_preserves_facts(&core);
    assert_eq!(
        core.deblocking_filter_params, None,
        "the truncated deblocking structure leaves its field None"
    );
    assert_eq!(core.gdf_params, None);
    assert_eq!(core.cdef_params, None);
}

#[test]
fn frame_header_core_eof_inside_gdf_params_preserves_facts() {
    let mut seq = byte_aligned_filter_seq();
    seq.filter.enable_gdf = true; // gdf_params() reads bits instead of short-circuiting
    let mut bits = intra_body_up_to_filter_cluster();
    bits.bit(1); // apply_deblocking_filter[0]
    bits.bit(1); // apply_deblocking_filter[1]
    bits.bit(1); // apply_deblocking_filter[2] (NumPlanes 3, luma set)
    bits.bit(1); // apply_deblocking_filter[3]
    bits.bit(0); // df_delta_q_present[0]
    bits.bit(0); // df_delta_q_present[1]
    bits.bit(0); // df_delta_q_present[2]
    bits.bit(0); // df_delta_q_present[3] -> deblocking ends at bit 56 (byte boundary)
    assert_eq!(
        bits.bit_len(),
        56,
        "deblocking consumes exactly byte 6 so gdf starts on byte 7"
    );
    bits.bit(1); // gdf_frame_enable (byte 7) -> dropped
    let mut data = bits.into_bytes();
    data.truncate(7); // 56 bits: the gdf_frame_enable read overruns
    let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
    assert_truncated_filter_cluster_preserves_facts(&core);
    assert!(
        core.deblocking_filter_params.is_some(),
        "deblocking parsed before the gdf truncation must survive"
    );
    assert_eq!(
        core.gdf_params, None,
        "the truncated gdf structure stays None"
    );
    assert_eq!(core.cdef_params, None);
}

#[test]
fn frame_header_core_eof_inside_cdef_params_preserves_facts() {
    let mut seq = byte_aligned_filter_seq();
    seq.filter.enable_cdef = true; // cdef_params() reads bits instead of short-circuiting
    let mut bits = intra_body_up_to_filter_cluster();
    bits.bit(1); // apply_deblocking_filter[0]
    bits.bit(1); // apply_deblocking_filter[1]
    bits.bit(1); // apply_deblocking_filter[2]
    bits.bit(1); // apply_deblocking_filter[3]
    bits.bit(0); // df_delta_q_present[0]
    bits.bit(0); // df_delta_q_present[1]
    bits.bit(0); // df_delta_q_present[2]
    bits.bit(0); // df_delta_q_present[3] -> deblocking ends at bit 56 (byte boundary)
    bits.bit(1); // cdef_frame_enable (byte 7) -> dropped
    let mut data = bits.into_bytes();
    data.truncate(7); // 56 bits: the cdef_frame_enable read overruns
    let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
    assert_truncated_filter_cluster_preserves_facts(&core);
    assert!(
        core.deblocking_filter_params.is_some(),
        "deblocking parsed before the cdef truncation must survive"
    );
    assert_eq!(
        core.gdf_params.as_ref().map(|g| g.gdf_frame_enable),
        Some(false),
        "the frame-disabled gdf structure parsed (no bits) before the cdef truncation"
    );
    assert_eq!(
        core.cdef_params, None,
        "the truncated cdef structure stays None"
    );
}

#[test]
fn frame_header_core_intra_tail_parses_lr_ccso_and_tail_to_completion() {
    let mut seq = byte_aligned_filter_seq();
    seq.restoration.enable_restoration = true;
    seq.restoration.lr_uv_pc_wiener_disabled = true;
    seq.ccso.enable_ccso = true;
    let mut bits = intra_body_up_to_filter_cluster();
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    bits.ns(0, 4); // plane 0 tool_index -> RESTORE_NONE
    bits.ns(0, 2); // plane 1 tool_index -> RESTORE_NONE
    bits.ns(0, 2); // plane 2 tool_index -> RESTORE_NONE
    bits.bit(1); // ccso_frame_flag
    bits.bit(0); // ccso_planes[0]
    bits.bit(0); // ccso_planes[1]
    bits.bit(0); // ccso_planes[2]
    bits.bit(1); // tx_mode_select = 1 -> TX_MODE_SELECT
    bits.f(3, 2); // reduced_tx_set = 3
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
    assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
    let lr = core.lr_params.as_ref().unwrap();
    assert!(!lr.uses_lr);
    assert_eq!(lr.planes.len(), 3);
    assert!(
        lr.planes
            .iter()
            .all(|p| p.restoration_type == FrameRestorationType::None)
    );
    let ccso = core.ccso_params.as_ref().unwrap();
    assert_eq!(ccso.ccso_frame_flag, Some(true));
    assert_eq!(ccso.planes.len(), 3);
    assert!(ccso.planes.iter().all(|p| !p.ccso_planes));
    let tail = core.intra_tail.as_ref().expect("intra tail parsed");
    assert_eq!(tail.tx_mode, TxMode::Select);
    assert_eq!(tail.reduced_tx_set, 3);
    assert!(!tail.reference_select);
    assert!(!tail.skip_mode_present);
    assert!(!tail.allow_bawp);
    assert!(!tail.use_global_motion);
    assert!(!tail.film_grain.apply_grain);
}

#[test]
fn frame_header_core_frame_filters_on_parses_wienerns_bank() {
    let mut seq = byte_aligned_filter_seq();
    seq.restoration.enable_restoration = true;
    seq.restoration.lr_pc_wiener_disabled = true;
    seq.restoration.lr_uv_pc_wiener_disabled = true;
    seq.ccso.enable_ccso = true;
    let mut bits = intra_body_up_to_filter_cluster();
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    bits.ns(1, 2); // plane 0 -> RESTORE_WIENER_NONSEP
    bits.bit(1); // frame_filters_on[0] == 1
    bits.f(1, 3); // num_filter_classes_idx == 1 -> Decode_Num_Filter_Classes[1] == 2
    bits.ns(0, 2); // plane 1 -> RESTORE_NONE
    bits.ns(0, 2); // plane 2 -> RESTORE_NONE
    bits.bit(1); // lr_luma_use_half_size
    bits.bit(0); // class 1 match_index == 1
    bits.bit(1); // merged[0]
    bits.bit(1); // merged[1]
    bits.bit(1); // ccso_frame_flag
    bits.bit(0); // ccso_planes[0]
    bits.bit(0); // ccso_planes[1]
    bits.bit(0); // ccso_planes[2]
    bits.bit(1); // tx_mode_select = 1 -> TX_MODE_SELECT
    bits.f(3, 2); // reduced_tx_set = 3
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
    assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
    assert_eq!(core.frame_size, Some(FrameSize::new(16, 16)));
    assert!(core.deblocking_filter_params.is_some());
    let lr = core.lr_params.as_ref().expect("lr_params parsed fully");
    assert!(lr.uses_lr, "luma RESTORE_WIENER_NONSEP uses LR");
    assert_eq!(lr.planes.len(), 3);
    assert_eq!(
        lr.planes[0].restoration_type,
        FrameRestorationType::WienerNonsep
    );
    assert!(lr.planes[0].frame_filters_on);
    assert_eq!(lr.planes[0].num_filter_classes, Some(2));
    let bank = lr.planes[0]
        .frame_filter_bank
        .as_ref()
        .expect("frame_filters_on carries the parsed bank");
    assert_eq!(bank.classes.len(), 2);
    assert_eq!(bank.classes[1].match_index, 1);
    assert!(bank.classes.iter().all(|class| class.merged));
    assert_eq!(lr.planes[1].restoration_type, FrameRestorationType::None);
    assert!(!lr.planes[1].frame_filters_on);
    assert_eq!(lr.loop_restoration_size[0], 256);
    assert!(core.ccso_params.is_some());
    assert!(core.intra_tail.is_some());
}

#[test]
fn frame_header_core_eof_inside_ccso_params_preserves_facts() {
    let mut seq = byte_aligned_filter_seq();
    seq.ccso.enable_ccso = true;
    let mut bits = intra_body_up_to_filter_cluster();
    bits.bit(1); // apply_deblocking_filter[0]
    bits.bit(1); // apply_deblocking_filter[1]
    bits.bit(1); // apply_deblocking_filter[2]
    bits.bit(1); // apply_deblocking_filter[3]
    bits.bit(0); // df_delta_q_present[0]
    bits.bit(0); // df_delta_q_present[1]
    bits.bit(0); // df_delta_q_present[2]
    bits.bit(0); // df_delta_q_present[3] -> deblocking ends at bit 56 (byte boundary)
    bits.bit(1); // ccso_frame_flag (byte 7) -> dropped by truncation
    let mut data = bits.into_bytes();
    data.truncate(7); // 56 bits: the ccso_frame_flag read overruns
    let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
    assert_truncated_filter_cluster_preserves_facts(&core);
    assert!(
        core.deblocking_filter_params.is_some(),
        "deblocking parsed before the ccso truncation must survive"
    );
    assert!(
        core.lr_params.is_some(),
        "the restoration-disabled lr structure parsed (no bits) before the ccso truncation"
    );
    assert_eq!(
        core.ccso_params, None,
        "the truncated ccso structure stays None"
    );
}

#[test]
fn frame_header_core_eof_inside_intra_tail_preserves_cluster_facts() {
    let mut seq = byte_aligned_filter_seq();
    seq.ccso.enable_ccso = true;
    let mut bits = intra_body_up_to_filter_cluster();
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    bits.bit(1); // ccso_frame_flag
    bits.bit(0); // ccso_planes[0]
    bits.bit(0); // ccso_planes[1]
    bits.bit(0); // ccso_planes[2]
    bits.bit(0); // tx_mode_select = 0 -> Largest
    bits.bit(0); // 1 of 2 reduced_tx_set bits; the next bit is missing
    let total_bits = bits.bit_len();
    let mut data = bits.into_bytes();
    let keep_bytes = total_bits / 8; // drop the partial trailing byte -> reduced_tx_set overruns
    data.truncate(keep_bytes);
    let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
    assert_eq!(core.status, FrameHeaderParseStatus::StoppedInsideIntraTail);
    assert_eq!(core.frame_size, Some(FrameSize::new(16, 16)));
    assert!(core.deblocking_filter_params.is_some());
    assert!(core.lr_params.is_some());
    assert!(core.ccso_params.is_some());
    assert_eq!(core.intra_tail, None, "the truncated tail stays None");
}

#[test]
fn frame_header_core_intra_tail_with_grain_present_reads_id_and_seed() {
    let mut seq = base_seq();
    seq.film_grain_params_present = Some(true);
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(1); // immediate_output_frame == 1 (output frame -> apply_grain readable)
    bits.bit(0); // frame_size_override_flag == 0 (cur_mfh_id == 0 -> max dims)
    bits.f(3, 4); // order_hint f(4)
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    bits.bit(1); // uniform_tile_spacing_flag (4096x2304 single uniform tile)
    bits.bit(0); // increment_tile_cols_log2 = 0
    bits.bit(0); // increment_tile_rows_log2 = 0
    bits.f(90, 8); // base_q_idx (non-lossless)
    bits.bit(0); // segmentation_enabled
    bits.bit(0); // using_qmatrix
    bits.bit(0); // delta_q_present
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    bits.bit(1); // tx_mode_select = 1 -> Select
    bits.f(1, 2); // reduced_tx_set = 1
    bits.bit(1); // apply_grain = 1
    bits.f(4, 3); // fgm_id = 4
    bits.f(0xC0DE, 16); // grain_seed
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
    assert_eq!(core.immediate_output_frame, Some(true));
    assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
    let tail = core.intra_tail.as_ref().unwrap();
    assert_eq!(tail.tx_mode, TxMode::Select);
    assert_eq!(tail.reduced_tx_set, 1);
    assert!(tail.film_grain.apply_grain);
    assert_eq!(tail.film_grain.fgm_id, Some(4));
    assert_eq!(tail.film_grain.grain_seed, Some(0xC0DE));
}

#[test]
fn frame_header_core_intra_unknown_grain_flag_parses_control_region_then_stops() {
    let mut seq = base_seq();
    seq.film_grain_params_present = None;
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(1); // immediate_output_frame == 1
    bits.bit(0); // frame_size_override_flag == 0 (cur_mfh_id == 0 -> max dims 4096x2304)
    bits.f(3, 4); // order_hint f(4)
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    bits.bit(1); // uniform_tile_spacing_flag
    bits.bit(0); // increment_tile_cols_log2 = 0
    bits.bit(0); // increment_tile_rows_log2 = 0
    bits.f(90, 8); // base_q_idx (non-lossless)
    bits.bit(0); // segmentation_enabled
    bits.bit(0); // using_qmatrix
    bits.bit(0); // delta_q_present
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
    assert_eq!(core.immediate_output_frame, Some(true));
    assert_eq!(core.order_hint_lsb, Some(3));
    assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
    assert!(core.quantization_params.is_some());
    assert!(core.deblocking_filter_params.is_some());
    assert!(matches!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
    ));
    assert_ne!(core.status, FrameHeaderParseStatus::ActivationFieldsOnly);
    assert_eq!(
        core.intra_tail, None,
        "the grain-gated tail was not reached"
    );
    assert!(
        !core.status.is_truncated_in_modeled_region(),
        "an unknown-flag stop is a coverage stop, not a truncation defect"
    );
}

#[test]
fn frame_header_core_sef_unknown_grain_flag_preserves_facts_then_stops() {
    let mut seq = base_seq();
    seq.film_grain_params_present = None;
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(6, 3); // frame_to_show_map_idx
    bits.bit(0); // derive_sef_order_hint == 0
    bits.f(11, 4); // sef_order_hint f(4)
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::RegularSef, true, &seq).unwrap();
    assert_eq!(core.show_existing_frame, Some(true));
    assert_eq!(core.frame_to_show_map_idx, Some(6));
    assert_eq!(core.derive_sef_order_hint, Some(false));
    assert_eq!(core.order_hint_lsb, Some(11));
    assert_eq!(core.refresh_frame_flags, Some(0));
    assert!(matches!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
    ));
    assert_ne!(core.status, FrameHeaderParseStatus::ActivationFieldsOnly);
    assert_eq!(
        core.sef_film_grain, None,
        "grain not decided without the flag"
    );
    assert_eq!(
        core.sef_trailing_bits, None,
        "no completed SEF tail to classify"
    );
}

#[test]
fn frame_header_core_sef_with_grain_reads_apply_grain_then_completes() {
    let mut seq = base_seq();
    seq.film_grain_params_present = Some(true);
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(6, 3); // frame_to_show_map_idx
    bits.bit(1); // derive_sef_order_hint == 1 -> no sef_order_hint
    bits.bit(1); // apply_grain = 1
    bits.f(2, 3); // fgm_id = 2
    bits.f(0x1357, 16); // grain_seed
    bits.bit(1); // trailing_one_bit
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::RegularSef, true, &seq).unwrap();
    assert_eq!(core.show_existing_frame, Some(true));
    assert_eq!(core.derive_sef_order_hint, Some(true));
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::ShowExistingFrameComplete
    );
    let fg = core.sef_film_grain.expect("SEF film_grain_config parsed");
    assert!(fg.apply_grain);
    assert_eq!(fg.fgm_id, Some(2));
    assert_eq!(fg.grain_seed, Some(0x1357));
    assert_eq!(core.sef_trailing_bits, Some(SefTrailingBits::Valid));
}

#[test]
fn frame_header_core_sef_eof_inside_film_grain_preserves_facts() {
    let mut seq = base_seq();
    seq.film_grain_params_present = Some(true);
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(6, 3); // frame_to_show_map_idx
    bits.bit(1); // derive_sef_order_hint == 1
    bits.bit(1); // apply_grain = 1
    bits.f(2, 3); // fgm_id = 2
    bits.f(0, 8); // only 8 of 16 grain_seed bits, then EOF
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::RegularSef, true, &seq).unwrap();
    assert_eq!(core.show_existing_frame, Some(true));
    assert_eq!(core.frame_to_show_map_idx, Some(6));
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::StoppedInsideShowExistingFrame
    );
    assert!(
        core.status.is_truncated_in_modeled_region(),
        "an EOF in the SEF film_grain_config() tail is a truncation in a modeled region"
    );
    assert_eq!(
        core.sef_film_grain, None,
        "the truncated SEF grain stays None"
    );
    assert_eq!(core.sef_trailing_bits, None);
}

#[test]
fn frame_header_core_sef_nonzero_bits_after_fields_flag_trailing_bits() {
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(6, 3); // frame_to_show_map_idx
    bits.bit(1); // derive_sef_order_hint == 1 -> no sef_order_hint
    bits.bit(0); // would-be trailing_one_bit, but it is 0
    bits.f(0b1011, 4); // arbitrary nonzero bits after the SEF fields
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::RegularSef, true, &base_seq()).unwrap();
    assert_eq!(core.show_existing_frame, Some(true));
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::ShowExistingFrameComplete,
        "the SEF fields still parse to completion; the defect is in the tail"
    );
    assert_eq!(
        core.sef_trailing_bits,
        Some(SefTrailingBits::MissingOneBit),
        "the first post-field bit was not the required trailing_one_bit"
    );
}

#[test]
fn frame_header_core_sef_grain_seed_short_one_bit_eats_trailing_marker() {
    let mut seq = base_seq();
    seq.film_grain_params_present = Some(true);
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(6, 3); // frame_to_show_map_idx
    bits.bit(1); // derive_sef_order_hint == 1
    bits.bit(1); // apply_grain = 1
    bits.f(2, 3); // fgm_id = 2
    bits.f(0x0000, 15); // 15 seed bits
    bits.bit(1); // the marker bit, consumed as the 16th grain_seed bit
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::RegularSef, true, &seq).unwrap();
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::ShowExistingFrameComplete
    );
    let fg = core.sef_film_grain.expect("SEF film_grain_config parsed");
    assert_eq!(fg.grain_seed, Some(1));
    assert_ne!(
        core.sef_trailing_bits,
        Some(SefTrailingBits::Valid),
        "the eaten trailing_one_bit makes the SEF tail non-conformant"
    );
    assert!(matches!(
        core.sef_trailing_bits,
        Some(SefTrailingBits::MissingOneBit | SefTrailingBits::Empty)
    ));
}

#[test]
fn frame_header_core_unresolvable_mfh_default_size_stays_unsupported() {
    let mut bits = Bits::default();
    bits.uvlc(2); // cur_mfh_id == 2 -> no seq_header_id_in_frame_header
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(0); // frame_size_override_flag == 0 (default dims)
    bits.f(7, 4); // order_hint
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap();

    assert_eq!(core.cur_mfh_id.get(), 2);
    assert_eq!(core.seq_header_id_in_frame_header, None);
    assert_eq!(core.order_hint_lsb, Some(7));
    assert_eq!(
        core.frame_size, None,
        "unresolvable cur_mfh_id > 0 default dims stay unknown"
    );
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: "AV2-5.18.2-FRAME-HEADER-INFO"
        }
    );
    assert_eq!(core.tile_info, None);
    assert_eq!(core.quantization_params, None);
}

#[test]
fn frame_header_core_unresolvable_mfh_with_explicit_size_stops_before_segmentation() {
    let mut bits = Bits::default();
    bits.uvlc(1); // cur_mfh_id == 1 -> no seq_header_id_in_frame_header
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(1); // frame_size_override_flag == 1 (explicit dims)
    bits.f(7, 4); // order_hint
    bits.f(1920 - 1, 12); // frame_width_minus_1
    bits.f(1080 - 1, 12); // frame_height_minus_1
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    bits.bit(1); // uniform_tile_spacing_flag (tile_info, single tile)
    bits.bit(0); // increment_tile_cols_log2 = 0
    bits.bit(0); // increment_tile_rows_log2 = 0
    bits.f(70, 8); // base_q_idx (quantization_params)
    let data = bits.into_bytes();
    let (core, consumed) = parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap();

    assert_eq!(core.cur_mfh_id.get(), 1);
    assert_eq!(core.frame_size, Some(FrameSize::new(1920, 1080)));
    assert_eq!(
        core.frame_size_override_flag,
        Some(true),
        "the override path records frame_size_override_flag == 1 (explicit dims provenance)"
    );
    assert_eq!(core.tile_info.as_ref().unwrap().tile_cols, 1);
    assert_eq!(core.quantization_params.unwrap().base_q_idx, 70);
    assert_eq!(core.segmentation_params, None);
    assert_eq!(core.setup_qm_params, None);
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: "AV2-5.18.2-FRAME-HEADER-INFO"
        }
    );
    assert_eq!(consumed, 3 + 33 + 3 + 8);
}

#[test]
fn frame_header_core_mfh_default_dims_parse_through_tile_info() {
    let mfh_size = Some(crate::hls::MfhFrameSize {
        width_bits: 12,
        height_bits: 12,
        width_minus_1: 1920 - 1,
        height_minus_1: 1080 - 1,
    });
    let record = mfh_record(mfh_size, None); // mfh_seg_info_present_flag == 0
    let view = MfhFrameView::from_record(&record, &base_seq());

    let mut bits = Bits::default();
    bits.uvlc(1); // cur_mfh_id == 1
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(0); // frame_size_override_flag == 0 (MFH default dims, no bits)
    bits.f(7, 4); // order_hint
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    bits.bit(1); // uniform_tile_spacing_flag (single tile)
    bits.bit(0); // increment_tile_cols_log2 = 0
    bits.bit(0); // increment_tile_rows_log2 = 0
    bits.f(70, 8); // base_q_idx
    bits.bit(0); // segmentation_enabled
    bits.bit(0); // using_qmatrix (setup_qm_params)
    bits.bit(0); // delta_q_present (base_q_idx 70 > 0; 0 -> no further delta_q bits)
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    bits.bit(0); // tx_mode_select = 0
    bits.f(0, 2); // reduced_tx_set = 0
    let data = bits.into_bytes();
    let (core, _) = parse_body_with_mfh(
        &data,
        ObuType::ClosedLoopKey,
        true,
        &base_seq(),
        Some(&view),
    )
    .unwrap();

    assert_eq!(core.cur_mfh_id.get(), 1);
    assert_eq!(
        core.frame_size,
        Some(FrameSize::new(1920, 1080)),
        "MFH default dims drive frame_size on the non-override path"
    );
    assert_eq!(
        core.frame_size_override_flag,
        Some(false),
        "the non-override default path records frame_size_override_flag == 0 (MFH-default provenance)"
    );
    assert_eq!(core.tile_info.as_ref().unwrap().tile_cols, 1);
    assert_eq!(core.quantization_params.unwrap().base_q_idx, 70);
    assert!(!core.segmentation_params.unwrap().segmentation_enabled);
    assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
}

#[test]
fn frame_header_core_mfh_omitted_size_infers_sequence_maxima() {
    let record = mfh_record(None, None); // no mfh_frame_size, no seg info
    let view = MfhFrameView::from_record(&record, &base_seq());
    assert_eq!(view.default_dims, (4096, 2304));

    let mut bits = Bits::default();
    bits.uvlc(1); // cur_mfh_id == 1
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(0); // frame_size_override_flag == 0 (MFH default = inferred maxima)
    bits.f(7, 4); // order_hint
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    bits.bit(1); // uniform_tile_spacing_flag
    bits.bit(0); // increment_tile_cols_log2
    bits.bit(0); // increment_tile_rows_log2
    bits.f(0, 8); // base_q_idx == 0 (no delta_q bits)
    bits.bit(0); // segmentation_enabled
    bits.bit(0); // using_qmatrix
    bits.f(0, 2); // reduced_tx_set = 0 (no tx_mode_select bit on the lossless gate)
    let data = bits.into_bytes();
    let (core, _) = parse_body_with_mfh(
        &data,
        ObuType::ClosedLoopKey,
        true,
        &base_seq(),
        Some(&view),
    )
    .unwrap();

    assert_eq!(
        core.frame_size,
        Some(FrameSize::new(4096, 2304)),
        "omitted MFH size infers the sequence maxima (:4101)"
    );
    assert!(core.lossless_info.as_ref().unwrap().coded_lossless);
    assert_eq!(
        core.deblocking_filter_params
            .unwrap()
            .apply_deblocking_filter,
        [false; 4]
    );
    assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
    let tail = core.intra_tail.as_ref().unwrap();
    assert_eq!(tail.tx_mode, TxMode::Only4x4);
    assert_eq!(tail.reduced_tx_set, 0);
}

#[test]
fn frame_header_core_mfh_segmentation_arm_reuses_mfh_feature_data() {
    let mut mfh_features = [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS];
    mfh_features[3][0] = SegmentFeature {
        enabled: true,
        data: 7,
    };
    let record = mfh_record(
        None,
        Some(&(
            false, // mfh_ext_seg_flag == enable_ext_seg (base_seq enable_ext_seg = false)
            false, // mfh_allow_seg_info_change
            SegmentInfo {
                num_segments: 8,
                features: mfh_features,
            },
        )),
    );
    let view = MfhFrameView::from_record(&record, &base_seq());

    let mut bits = Bits::default();
    bits.uvlc(1); // cur_mfh_id == 1
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(1); // frame_size_override_flag == 1
    bits.f(7, 4); // order_hint
    bits.f(1920 - 1, 12); // frame_width_minus_1
    bits.f(1080 - 1, 12); // frame_height_minus_1
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    bits.bit(1); // uniform_tile_spacing_flag
    bits.bit(0); // increment_tile_cols_log2
    bits.bit(0); // increment_tile_rows_log2
    bits.f(70, 8); // base_q_idx
    bits.bit(1); // segmentation_enabled
    bits.bit(0); // using_qmatrix
    bits.bit(0); // delta_q_present
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    bits.bit(0); // tx_mode_select = 0
    bits.f(0, 2); // reduced_tx_set = 0
    let data = bits.into_bytes();
    let (core, _) = parse_body_with_mfh(
        &data,
        ObuType::ClosedLoopKey,
        true,
        &base_seq(),
        Some(&view),
    )
    .unwrap();

    let seg = core
        .segmentation_params
        .expect("segmentation parsed on MFH arm");
    assert!(seg.segmentation_enabled);
    assert!(
        seg.reuse_seg_info,
        "MFH arm with allowChange==0 infers reuse"
    );
    assert!(
        seg.features[3][0].enabled,
        "reuse copies MfhFeatureEnabled/MfhFeatureData, not sequence data"
    );
    assert_eq!(seg.features[3][0].data, 7);
    assert_eq!(seg.last_active_seg_id, 3);
    assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
    assert!(core.intra_tail.is_some());
}

#[test]
fn frame_header_core_intra_tail_parses_full_structure_cluster() {
    let mut seq = base_seq();
    seq.quant.choose_tcq_per_frame = true;
    seq.quant.enable_parity_hiding = true;
    seq.filter.enable_gdf = true;
    seq.filter.enable_cdef = true;
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(1); // frame_size_override_flag
    bits.f(5, 4); // order_hint
    bits.f(1920 - 1, 12); // frame_width_minus_1
    bits.f(1080 - 1, 12); // frame_height_minus_1
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    bits.bit(1); // uniform_tile_spacing_flag
    bits.bit(1); // increment_tile_cols_log2 = 1
    bits.bit(0); // increment_tile_cols_log2 = 0
    bits.bit(0); // increment_tile_rows_log2 = 0
    bits.f(1, 1); // context_update_tile_id f(TileRowsLog2 + TileColsLog2 == 1)
    bits.f(3, 2); // tile_size_bytes_minus_1 -> TileSizeBytes = 4
    bits.f(40, 8); // base_q_idx
    bits.bit(1); // segmentation_enabled
    for _ in 0..8 {
        bits.f(0, 3); // seg_info: feature_enabled[i][0..3] = 0
    }
    bits.bit(1); // using_qmatrix
    bits.f(1, 2); // pic_qm_num_minus_1 -> qmNum = 2
    bits.f(3, 4); // qm_y[0]
    bits.bit(1); // qm_uv_same_as_y[0]
    bits.f(5, 4); // qm_y[1]
    bits.bit(1); // qm_uv_same_as_y[1]
    bits.bit(0); // delta_q_present
    for _ in 0..8 {
        bits.bit(1); // qm_index
    }
    bits.bit(0); // allow_tcq (choose_tcq_per_frame)
    bits.bit(1); // allow_parity_hiding
    bits.bit(1); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    bits.bit(0); // apply_deblocking_filter[2]
    bits.bit(0); // apply_deblocking_filter[3]
    bits.bit(1); // df_delta_q_present[0]
    bits.f(3, 2); // df_delta_q[0]
    bits.bit(1); // gdf_frame_enable
    bits.bit(0); // gdf_per_block
    bits.f(2, 2); // gdf_pic_qc_idx
    bits.f(3, 2); // gdf_pic_scale_idx -> GdfPixScale = 4
    bits.bit(1); // cdef_frame_enable
    bits.f(1, 2); // cdef_damping_minus_3 -> CdefDamping = 4
    bits.f(0, 3); // cdef_strengths_minus_1 -> CdefStrengths = 1
    bits.bit(1); // cdef_on_skip_txfm_frame_enable (adaptive -> read)
    bits.bit(0); // cdef_y_pri_zero -> read f(4)
    bits.f(9, 4); // cdef_y_pri_strength[0]
    bits.f(1, 2); // cdef_y_sec_strength[0]
    bits.bit(1); // cdef_uv_pri_zero -> 0
    bits.f(3, 2); // cdef_uv_sec_strength[0] == 3 -> 4
    bits.bit(1); // tx_mode_select = 1 -> TX_MODE_SELECT
    bits.f(2, 2); // reduced_tx_set = 2
    let data = bits.into_bytes();
    let (core, consumed) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();

    assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
    let tile_info = core.tile_info.as_ref().unwrap();
    assert_eq!(tile_info.tile_cols, 2);
    assert_eq!(tile_info.tile_rows, 1);
    assert_eq!(tile_info.tile_cols_log2, 1);
    assert_eq!(tile_info.mi_col_starts, vec![0, 256, 480]);
    assert_eq!(tile_info.mi_row_starts, vec![0, 270]);
    assert_eq!(tile_info.context_update_tile_id, 1);
    assert_eq!(tile_info.tile_size_bytes, Some(4));
    assert_eq!(core.quantization_params.unwrap().base_q_idx, 40);
    let segmentation = core.segmentation_params.unwrap();
    assert!(segmentation.segmentation_enabled);
    assert!(segmentation.segmentation_update_map);
    assert!(!segmentation.segmentation_temporal_update);
    let qm = core.setup_qm_params.unwrap();
    assert!(qm.using_qmatrix);
    assert_eq!(qm.pic_qm_num_minus_1, 1);
    assert_eq!(qm.levels[0].qm_y, 3);
    assert_eq!(qm.levels[1].qm_y, 5);
    assert!(!core.delta_q_params.unwrap().delta_q_present);
    let lossless = core.lossless_info.unwrap();
    assert!(!lossless.coded_lossless);
    assert!(!lossless.has_lossless_segment);
    assert!(lossless.seg_qm_levels[..8].iter().all(|l| *l == [5, 5, 5]));
    assert!(!lossless.allow_tcq);
    assert!(lossless.allow_parity_hiding);
    let deblocking = core.deblocking_filter_params.unwrap();
    assert_eq!(
        deblocking.apply_deblocking_filter,
        [true, false, false, false]
    );
    assert_eq!(deblocking.df_delta_q_present, [true, false, false, false]);
    assert_eq!(deblocking.df_delta_q, [1, 0, 0, 0]);
    let gdf = core.gdf_params.unwrap();
    assert!(gdf.gdf_frame_enable);
    assert_eq!(gdf.gdf_per_block, Some(false));
    assert_eq!(gdf.gdf_pic_qc_idx, Some(2));
    assert_eq!(gdf.gdf_pic_scale_idx, Some(3));
    let cdef = core.cdef_params.unwrap();
    assert!(cdef.cdef_frame_enable);
    assert_eq!(cdef.cdef_damping, Some(4));
    assert_eq!(cdef.cdef_strengths, Some(1));
    assert_eq!(cdef.cdef_on_skip_txfm_frame_enable, Some(true));
    assert_eq!(cdef.strengths.len(), 1);
    assert_eq!(cdef.strengths[0].y_pri_strength, 9);
    assert_eq!(cdef.strengths[0].y_sec_strength, 1);
    assert_eq!(cdef.strengths[0].uv_pri_strength, 0);
    assert_eq!(cdef.strengths[0].uv_sec_strength, 4);
    let lr = core.lr_params.as_ref().unwrap();
    assert!(!lr.uses_lr);
    assert!(lr.planes.is_empty());
    let ccso = core.ccso_params.as_ref().unwrap();
    assert_eq!(ccso.ccso_frame_flag, None);
    assert!(ccso.planes.is_empty());
    let tail = core.intra_tail.as_ref().unwrap();
    assert_eq!(tail.tx_mode, TxMode::Select);
    assert_eq!(tail.reduced_tx_set, 2);
    assert!(!tail.film_grain.apply_grain);
    assert_eq!(consumed, 2 + 33 + 64 + 30 + 3);
}

#[test]
fn frame_header_core_eof_inside_intra_structures() {
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(1); // frame_size_override_flag
    bits.f(5, 4); // order_hint
    bits.f(1920 - 1, 12); // frame_width_minus_1
    bits.f(1080 - 1, 12); // frame_height_minus_1
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    let data = bits.into_bytes();
    let err = parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap_err();
    assert!(matches!(err, Error::UnexpectedEof { .. }));
}

#[test]
fn frame_header_core_intra_only_reads_refresh_frame_flags() {
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(0); // frame_is_inter == 0 -> INTRA_ONLY
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(0); // frame_size_override_flag == 0 (cur_mfh_id == 0 -> max dims)
    bits.f(3, 4); // order_hint
    bits.f(0b0000_0101, 8); // refresh_frame_flags f(NumRefFrames == 8)
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    bits.bit(1); // uniform_tile_spacing_flag
    bits.bit(0); // increment_tile_cols_log2 = 0
    bits.bit(0); // increment_tile_rows_log2 = 0
    bits.f(45, 8); // base_q_idx
    bits.bit(0); // segmentation_enabled
    bits.bit(0); // using_qmatrix
    bits.bit(0); // delta_q_present
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    bits.bit(0); // tx_mode_select = 0
    bits.f(0, 2); // reduced_tx_set = 0
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::RegularTileGroup, true, &base_seq()).unwrap();

    assert_eq!(core.frame_type, Some(FrameType::IntraOnly));
    assert_eq!(core.frame_is_intra, Some(true));
    assert_eq!(core.refresh_frame_flags, Some(0b0000_0101));
    assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
    assert_eq!(core.quantization_params.unwrap().base_q_idx, 45);
    assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
    assert!(core.intra_tail.is_some());
}

#[test]
fn frame_header_core_single_picture_path() {
    let mut seq = base_seq();
    seq.single_picture_header_flag = true;
    seq.filter.single_picture_header_flag = true;
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(9, 4); // order_hint
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    bits.bit(1); // uniform_tile_spacing_flag
    bits.bit(0); // increment_tile_cols_log2 = 0
    bits.bit(0); // increment_tile_rows_log2 = 0
    bits.f(45, 8); // base_q_idx
    bits.bit(0); // segmentation_enabled
    bits.bit(0); // using_qmatrix
    bits.bit(0); // delta_q_present
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    bits.bit(0); // tx_mode_select = 0 -> TX_MODE_LARGEST
    bits.f(1, 2); // reduced_tx_set = 1
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();

    assert_eq!(core.show_existing_frame, Some(false));
    assert_eq!(core.frame_type, Some(FrameType::Key));
    assert_eq!(core.immediate_output_frame, Some(true));
    assert_eq!(core.implicit_output_frame, Some(false));
    assert_eq!(core.order_hint_lsb, Some(9));
    assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
    assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
    let tail = core.intra_tail.as_ref().expect("intra tail parsed");
    assert_eq!(tail.tx_mode, TxMode::Largest);
    assert_eq!(tail.reduced_tx_set, 1);
    assert!(!tail.film_grain.apply_grain);
}

#[test]
fn frame_header_core_single_picture_bridge_reads_prefix_then_bridge_return() {
    let mut seq = base_seq();
    seq.single_picture_header_flag = true;
    seq.filter.single_picture_header_flag = true;
    let mut bits = Bits::default();
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(5, 3); // bridge_frame_ref_idx = 5 f(CeilLog2(8) == 3) — read before single-pic
    bits.bit(0); // bridge_frame_overwrite_flag = 0 f(1) (mirror :4423)
    bits.f(32 - 1, 12); // bridge_frame_width_minus_1 (§ 5.18.4.2)
    bits.f(48 - 1, 12); // bridge_frame_height_minus_1 (§ 5.18.4.2)
    bits.bit(0); // allow_intrabc = 0 f(1) (intrabc_params(), mirror :4571)
    let data = bits.into_bytes();
    let mut valid = [false; NUM_REF_FRAMES];
    valid[5] = true;
    let mut hints = [0; NUM_REF_FRAMES];
    hints[5] = 9;
    let mut widths = [0; NUM_REF_FRAMES];
    widths[5] = 64;
    let mut heights = [0; NUM_REF_FRAMES];
    heights[5] = 64;
    let mut base_q = [0; NUM_REF_FRAMES];
    base_q[5] = 21;
    let chroma_deltas = [[0; 2]; NUM_REF_FRAMES];
    let reference_state = FrameReferenceStateView::from_slots_with_base_q_idx(
        &valid, &hints, &widths, &heights, &base_q,
    )
    .with_quantizer_delta_state(&chroma_deltas);
    let (core, _) = parse_body_with_ref(
        &data,
        ObuType::BridgeFrame,
        true,
        &seq,
        None,
        &reference_state,
    )
    .unwrap();

    assert!(core.is_bridge, "the OBU is still an OBU_BRIDGE_FRAME");
    assert_eq!(
        core.bridge_frame_ref_idx,
        Some(5),
        "bridge_frame_ref_idx is read before the single-picture branch (mirror :4117)"
    );
    assert_eq!(core.show_existing_frame, Some(false));
    assert_eq!(core.frame_type, Some(FrameType::Key));
    assert_eq!(core.frame_is_intra, Some(true));
    assert_eq!(
        core.immediate_output_frame,
        Some(true),
        "single_picture forces immediate_output_frame = 1"
    );
    assert_eq!(core.implicit_output_frame, Some(false));
    let inter = core
        .inter
        .as_ref()
        .expect("a single-picture bridge records its IsBridge facts on core.inter");
    assert_eq!(
        inter.bridge_frame_overwrite_flag,
        Some(false),
        "bridge_frame_overwrite_flag f(1) IS read (mirror :4423) — the pre-fix intra path did not"
    );
    assert_eq!(
        inter.refresh_frame_flags,
        Some(1 << 5),
        "overwrite == 0 -> refresh inferred 1 << bridge_frame_ref_idx (§ 6.17.2 + AVM, no bits)"
    );
    assert_eq!(
        inter.num_total_refs,
        Some(0),
        "the FrameIsIntra arm sets NumTotalRefs = 0 (mirror :4573)"
    );
    assert_eq!(
        inter.primary_ref_frame,
        Some(7),
        "PRIMARY_REF_NONE (mirror :4345)"
    );
    assert_eq!(core.refresh_frame_flags, Some(1 << 5));
    assert_eq!(core.frame_size, Some(FrameSize::new(32, 48)));
    assert_eq!(core.allow_screen_content_tools, Some(false));
    assert_eq!(core.allow_intrabc, Some(false));
    assert_eq!(core.disable_cdf_update, Some(true));
    assert!(core.tile_info.is_some());
    assert_eq!(
        core.quantization_params.map(|quant| quant.base_q_idx),
        Some(21)
    );
    assert!(
        core.intra_tail.is_none(),
        "the full intra tail is NOT taken for a single-picture bridge"
    );
    assert_eq!(
        inter.stop,
        Some(InterStop::BruInactiveOrBridgeReturn),
        "stops at the § 5.18.2 IsBridge early-return arm (mirror :4971), not IntraHeaderComplete"
    );
    assert_eq!(core.status, FrameHeaderParseStatus::InterHeaderComplete);
}

#[test]
fn frame_header_core_single_picture_bridge_reads_scc_and_intrabc_conditionals() {
    let mut seq = base_seq();
    seq.single_picture_header_flag = true;
    seq.filter.single_picture_header_flag = true;
    seq.enable_short_refresh_frame_flags = true; // overwrite==1 -> has_refresh + frame_to_refresh
    seq.seq_force_screen_content_tools = 2; // SELECT_SCREEN_CONTENT_TOOLS -> read the bit
    seq.seq_force_integer_mv = 2; // SELECT_INTEGER_MV -> read force_integer_mv when SCC on
    let mut bits = Bits::default();
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(5, 3); // bridge_frame_ref_idx = 5
    bits.bit(1); // bridge_frame_overwrite_flag = 1 (mirror :4423) -> refresh IS read
    bits.bit(1); // has_refresh_frame_flags = 1 (overwrite==1 short path)
    bits.f(5, 3); // frame_to_refresh = 5 f(CeilLog2(8) == 3) -> refresh = 1 << 5
    bits.f(64 - 1, 12); // bridge_frame_width_minus_1
    bits.f(64 - 1, 12); // bridge_frame_height_minus_1
    bits.bit(1); // allow_screen_content_tools = 1 (mirror :4569 / §5.18.3.3)
    bits.bit(1); // force_integer_mv = 1 (allow_sct && seq_force_integer_mv == SELECT)
    bits.bit(1); // allow_intrabc = 1 (mirror :4571 / §5.18.3.4)
    bits.bit(1); // allow_global_intrabc = 1 (allow_intrabc && FrameIsIntra)
    bits.bit(0); // allow_local_intrabc = 0 (allow_global_intrabc == 1 -> read)
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::BridgeFrame, true, &seq).unwrap();

    let inter = core.inter.as_ref().expect("bridge facts recorded");
    assert_eq!(inter.bridge_frame_overwrite_flag, Some(true));
    assert_eq!(
        inter.refresh_frame_flags,
        Some(1 << 5),
        "overwrite == 1 + enable_short -> has_refresh_frame_flags + frame_to_refresh (1 << 5)"
    );
    assert_eq!(core.allow_screen_content_tools, Some(true));
    assert_eq!(core.force_integer_mv, Some(true));
    assert_eq!(core.allow_intrabc, Some(true));
    let intrabc = core.intrabc.as_ref().expect("intrabc params recorded");
    assert_eq!(intrabc.allow_global_intrabc, Some(true));
    assert_eq!(intrabc.allow_local_intrabc, Some(false));
    assert_eq!(inter.stop, Some(InterStop::BruInactiveOrBridgeReturn));
    assert!(matches!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
    ));
}

#[test]
fn frame_header_core_single_picture_bridge_eof_in_prefix_is_truncation() {
    let mut seq = base_seq();
    seq.single_picture_header_flag = true;
    seq.filter.single_picture_header_flag = true;
    let mut bits = Bits::default();
    bits.uvlc(0); // seq_header_id_in_frame_header (1 bit)
    bits.f(5, 3); // bridge_frame_ref_idx = 5 (3 bits)
    bits.bit(1); // bridge_frame_overwrite_flag = 1 (1 bit) -> 5 bits, padded to 1 byte
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::BridgeFrame, true, &seq).unwrap();

    assert_eq!(
        core.status,
        FrameHeaderParseStatus::StoppedInsideInterControl,
        "EOF inside the modeled bridge prefix is a facts-preserving truncation"
    );
    let inter = core
        .inter
        .as_ref()
        .expect("the pre-EOF facts are preserved on core.inter");
    assert_eq!(
        inter.bridge_frame_overwrite_flag,
        Some(true),
        "bridge_frame_overwrite_flag parsed before the EOF is preserved"
    );
    assert_eq!(
        inter.refresh_frame_flags, None,
        "refresh_frame_flags hit the EOF"
    );
    assert_eq!(inter.stop, None, "the bridge-return stop was never reached");
    assert_eq!(core.frame_size, None);
}

#[test]
fn frame_header_core_single_picture_bridge_reads_film_grain_tail() {
    let mut seq = base_seq();
    seq.single_picture_header_flag = true;
    seq.filter.single_picture_header_flag = true;
    seq.film_grain_params_present = Some(true);
    let mut bits = Bits::default();
    bits.uvlc(0); // seq_header_id_in_frame_header (1 bit)
    bits.f(5, 3); // bridge_frame_ref_idx = 5 (3 bits)
    bits.bit(0); // bridge_frame_overwrite_flag = 0 (1 bit) -> refresh inferred 1 << 5, no bits
    bits.f(64 - 1, 12); // bridge_frame_width_minus_1
    bits.f(64 - 1, 12); // bridge_frame_height_minus_1
    bits.bit(0); // allow_intrabc = 0 (1 bit)
    bits.f(5, 3); // fgm_id = 5
    bits.f(0xBEEF, 16); // grain_seed
    let data = bits.into_bytes();
    let mut valid = [false; NUM_REF_FRAMES];
    valid[5] = true;
    let hints = [0; NUM_REF_FRAMES];
    let widths = [64; NUM_REF_FRAMES];
    let heights = [64; NUM_REF_FRAMES];
    let base_q = [21; NUM_REF_FRAMES];
    let chroma_deltas = [[0; 2]; NUM_REF_FRAMES];
    let reference_state = FrameReferenceStateView::from_slots_with_base_q_idx(
        &valid, &hints, &widths, &heights, &base_q,
    )
    .with_quantizer_delta_state(&chroma_deltas);
    let (core, consumed) = parse_body_with_ref(
        &data,
        ObuType::BridgeFrame,
        true,
        &seq,
        None,
        &reference_state,
    )
    .unwrap();

    assert!(core.is_bridge);
    let inter = core.inter.as_ref().expect("bridge facts recorded");
    assert_eq!(
        inter.refresh_frame_flags,
        Some(1 << 5),
        "overwrite == 0 -> refresh inferred (no bits)"
    );
    assert_eq!(inter.stop, Some(InterStop::BruInactiveOrBridgeReturn));
    assert_eq!(core.status, FrameHeaderParseStatus::InterHeaderComplete);
    assert_eq!(
        consumed, 49,
        "consumed_bits covers bridge sizing and film grain"
    );
    assert_eq!(
        core.bridge_film_grain,
        Some(FilmGrainConfig {
            apply_grain: true,
            fgm_id: Some(5),
            grain_seed: Some(0xBEEF),
        })
    );
}

#[test]
fn frame_header_core_single_picture_bridge_waits_for_tile_state_before_film_grain() {
    let mut seq = base_seq();
    seq.single_picture_header_flag = true;
    seq.filter.single_picture_header_flag = true;
    seq.film_grain_params_present = Some(true);
    let mut bits = Bits::default();
    bits.uvlc(0); // seq_header_id (1 bit)
    bits.f(5, 3); // bridge_frame_ref_idx (3 bits) -> 4
    bits.bit(0); // bridge_frame_overwrite_flag = 0 (1 bit) -> 5; refresh inferred (no bits)
    bits.f(64 - 1, 12); // bridge_frame_width_minus_1
    bits.f(64 - 1, 12); // bridge_frame_height_minus_1
    bits.bit(0); // allow_intrabc (1 bit) -> 6
    bits.f(5, 3); // fgm_id f(3) -> 9; grain_seed f(16) then runs out of bits -> EOF.
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::BridgeFrame, true, &seq).unwrap();

    assert!(matches!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
    ));
    let inter = core.inter.as_ref().expect("pre-stop facts preserved");
    assert_eq!(inter.bridge_frame_overwrite_flag, Some(false));
    assert_eq!(
        inter.refresh_frame_flags,
        Some(1 << 5),
        "the inferred refresh is preserved before reference-derived tile state"
    );
    assert_eq!(
        core.frame_size, None,
        "unknown reference state prevents deriving the clamped bridge size"
    );
    assert_eq!(
        core.bridge_film_grain, None,
        "film grain follows tile_info and is not read without reference-derived tile state"
    );
}

#[test]
fn frame_header_core_bridge_parses_overwrite_refresh_and_size_arms() {
    let mut bits = Bits::default();
    bits.uvlc(4); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
    bits.f(5, 3); // bridge_frame_ref_idx = 5
    bits.bit(0); // bridge_frame_overwrite_flag = 0 -> refresh = 1 << 5 (no bits)
    bits.f(1920 - 1, 12); // bridge_frame_width_minus_1
    bits.f(1080 - 1, 12); // bridge_frame_height_minus_1
    let data = bits.into_bytes();

    let mut ref_valid = [false; NUM_REF_FRAMES];
    ref_valid[5] = true;
    let ref_oh = [0u32; NUM_REF_FRAMES];
    let mut ref_w = [0u32; NUM_REF_FRAMES];
    let mut ref_h = [0u32; NUM_REF_FRAMES];
    ref_w[5] = 1280;
    ref_h[5] = 720;
    let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);
    let (core, _) =
        parse_body_with_ref(&data, ObuType::BridgeFrame, true, &base_seq(), None, &rs).unwrap();

    assert!(core.is_bridge);
    assert_eq!(core.bridge_frame_ref_idx, Some(5));
    assert_eq!(core.frame_type, Some(FrameType::Inter));
    assert_eq!(core.frame_is_intra, Some(false));
    assert_eq!(core.immediate_output_frame, Some(false));
    assert_eq!(core.implicit_output_frame, Some(false));
    let inter = core.inter.as_ref().expect("bridge inter control parsed");
    assert_eq!(inter.bridge_frame_overwrite_flag, Some(false));
    assert_eq!(inter.refresh_frame_flags, Some(1 << 5));
    assert_eq!(inter.primary_ref_frame, Some(7)); // PRIMARY_REF_NONE
    assert_eq!(inter.explicit_ref_frame_map, Some(true));
    assert_eq!(inter.num_total_refs, Some(1));
    assert_eq!(inter.ref_frame_idx, vec![5]);
    assert_eq!(core.frame_size, Some(FrameSize::new(1280, 720)));
    assert_eq!(inter.stop, Some(InterStop::BruInactiveOrBridgeReturn));
    assert!(matches!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
    ));
}

#[test]
fn frame_header_core_bridge_overwrite_reads_refresh_frame_flags() {
    let mut bits = Bits::default();
    bits.uvlc(4); // seq_header_id_in_frame_header
    bits.f(5, 3); // bridge_frame_ref_idx = 5
    bits.bit(1); // bridge_frame_overwrite_flag = 1 -> refresh f(NumRefFrames)
    bits.f(0b1010_0101, 8); // refresh_frame_flags f(8)
    bits.f(1920 - 1, 12); // bridge_frame_width_minus_1
    bits.f(1080 - 1, 12); // bridge_frame_height_minus_1
    let data = bits.into_bytes();

    let mut ref_valid = [false; NUM_REF_FRAMES];
    ref_valid[5] = true;
    let ref_oh = [0u32; NUM_REF_FRAMES];
    let mut ref_w = [0u32; NUM_REF_FRAMES];
    let mut ref_h = [0u32; NUM_REF_FRAMES];
    ref_w[5] = 1280;
    ref_h[5] = 720;
    let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);
    let (core, _) =
        parse_body_with_ref(&data, ObuType::BridgeFrame, true, &base_seq(), None, &rs).unwrap();

    let inter = core.inter.as_ref().expect("bridge inter control parsed");
    assert_eq!(inter.bridge_frame_overwrite_flag, Some(true));
    assert_eq!(inter.refresh_frame_flags, Some(0b1010_0101));
    assert_eq!(core.frame_size, Some(FrameSize::new(1280, 720)));
    assert_eq!(inter.stop, Some(InterStop::BruInactiveOrBridgeReturn));
}

#[test]
fn frame_header_core_bridge_completes_with_reference_quantizer_state() {
    let mut bits = Bits::default();
    bits.uvlc(4); // seq_header_id_in_frame_header
    bits.f(5, 3); // bridge_frame_ref_idx = 5
    bits.bit(0); // bridge_frame_overwrite_flag = 0 -> refresh = 1 << 5
    bits.f(1920 - 1, 12); // bridge_frame_width_minus_1
    bits.f(1080 - 1, 12); // bridge_frame_height_minus_1
    let data = bits.into_bytes();

    let mut ref_valid = [false; NUM_REF_FRAMES];
    ref_valid[5] = true;
    let ref_oh = [0u32; NUM_REF_FRAMES];
    let mut ref_w = [0u32; NUM_REF_FRAMES];
    let mut ref_h = [0u32; NUM_REF_FRAMES];
    let mut ref_q = [0u32; NUM_REF_FRAMES];
    let mut ref_chroma_ac_deltas = [[0i32; 2]; NUM_REF_FRAMES];
    ref_w[5] = 1280;
    ref_h[5] = 720;
    ref_q[5] = 91;
    ref_chroma_ac_deltas[5] = [-3, 4];
    let rs = FrameReferenceStateView::from_slots_with_base_q_idx(
        &ref_valid, &ref_oh, &ref_w, &ref_h, &ref_q,
    )
    .with_quantizer_delta_state(&ref_chroma_ac_deltas);
    let (core, _) =
        parse_body_with_ref(&data, ObuType::BridgeFrame, true, &base_seq(), None, &rs).unwrap();

    assert_eq!(core.status, FrameHeaderParseStatus::InterHeaderComplete);
    assert_eq!(core.frame_size, Some(FrameSize::new(1280, 720)));
    assert!(core.tile_info.is_some());
    assert_eq!(
        core.quantization_params,
        Some(QuantizationParams::inferred_tip(91, -3, 4))
    );
    assert_eq!(
        core.bridge_film_grain,
        Some(FilmGrainConfig {
            apply_grain: false,
            fgm_id: None,
            grain_seed: None,
        })
    );
}

#[test]
fn frame_header_core_show_existing_frame_reads_map_idx_and_order_hint() {
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(6, 3); // frame_to_show_map_idx
    bits.bit(0); // derive_sef_order_hint == 0
    bits.f(11, 4); // sef_order_hint
    bits.bit(1); // § 5.2.3 trailing_one_bit; into_bytes() zero-pads the rest.
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::RegularSef, true, &base_seq()).unwrap();

    assert_eq!(core.show_existing_frame, Some(true));
    assert_eq!(core.frame_to_show_map_idx, Some(6));
    assert_eq!(core.order_hint_lsb, Some(11));
    assert_eq!(core.refresh_frame_flags, Some(0));
    assert_eq!(
        core.frame_type, None,
        "FrameType comes from reference state"
    );
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::ShowExistingFrameComplete
    );
    let fg = core.sef_film_grain.expect("SEF film_grain_config parsed");
    assert!(!fg.apply_grain);
    assert_eq!(fg.fgm_id, None);
    assert_eq!(core.sef_trailing_bits, Some(SefTrailingBits::Valid));
}

#[test]
fn frame_header_core_show_existing_frame_derives_order_hint() {
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(2, 3); // frame_to_show_map_idx
    bits.bit(1); // derive_sef_order_hint == 1 -> no sef_order_hint bits
    bits.bit(1); // § 5.2.3 trailing_one_bit; into_bytes() zero-pads the rest.
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::RegularSef, true, &base_seq()).unwrap();

    assert_eq!(core.show_existing_frame, Some(true));
    assert_eq!(core.frame_to_show_map_idx, Some(2));
    assert_eq!(
        core.order_hint_lsb, None,
        "order hint is derived from the slot, not signaled"
    );
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::ShowExistingFrameComplete
    );
    assert!(core.sef_film_grain.is_some());
    assert_eq!(core.sef_trailing_bits, Some(SefTrailingBits::Valid));
}

#[test]
fn frame_header_core_inter_implicit_map_stops_unmodeled() {
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(1); // frame_is_inter == 1
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(0); // frame_size_override_flag
    bits.f(5, 4); // order_hint f(OrderHintBits == 4)
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init (not TIP)
    bits.f(0, 8); // refresh_frame_flags f(NumRefFrames == 8)
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::RegularTileGroup, true, &base_seq()).unwrap();

    assert_eq!(core.frame_type, Some(FrameType::Inter));
    assert_eq!(core.frame_is_intra, Some(false));
    assert_eq!(core.immediate_output_frame, Some(false));
    assert_eq!(core.order_hint_lsb, Some(5));
    assert!(matches!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
    ));
    let inter = core.inter.as_ref().unwrap();
    assert_eq!(inter.explicit_ref_frame_map, Some(false));
    assert_eq!(
        inter.stop,
        Some(crate::headers::frame::inter::InterStop::UnmodeledDerivation)
    );
}

#[test]
fn frame_header_core_inter_explicit_map_reaches_shared_tail() {
    let mut seq = base_seq();
    seq.inter.explicit_ref_frame_map = true;
    seq.inter.enable_ref_frame_mvs = true;
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(1); // frame_is_inter == 1
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(0); // frame_size_override_flag
    bits.f(7, 4); // order_hint
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init
    bits.f(0, 8); // refresh_frame_flags
    bits.bit(1); // frame_explicit_ref_frame_map
    bits.f(1, 3); // num_total_refs = 1
    bits.f(2, 3); // ref_frame_idx[0]
    bits.bit(0); // use_ref_frame_mvs (num_total_refs == 1 -> no tmvp)
    bits.bit(0); // allow_intrabc
    bits.bit(0); // use_qtr_precision_mv
    bits.bit(0); // allow_high_precision_mv
    bits.bit(1); // is_filter_switchable
    bits.bit(0); // disable_cdf_update f(1) (mirror :5041), just before the shared tail.
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::RegularTileGroup, true, &seq).unwrap();

    assert_eq!(core.frame_type, Some(FrameType::Inter));
    let inter = core.inter.as_ref().unwrap();
    assert_eq!(inter.explicit_ref_frame_map, Some(true));
    assert_eq!(inter.num_total_refs, Some(1));
    assert_eq!(inter.ref_frame_idx, vec![2]);
    assert_eq!(inter.frame_size, Some(FrameSize::new(4096, 2304)));
    assert_eq!(inter.mv_precision, Some(MvPrecision::HalfPel));
    assert_eq!(
        inter.interpolation_filter,
        Some(InterpolationFilter::Switchable)
    );
    assert_eq!(inter.disable_cdf_update, Some(false));
    assert_eq!(core.disable_cdf_update, Some(false));
    assert_eq!(
        inter.stop,
        Some(crate::headers::frame::inter::InterStop::ReachedSharedTail)
    );
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::StoppedInsideInterControl
    );
    assert!(core.status.is_truncated_in_modeled_region());
}

/// A 64x64 minimal-tool inter sequence view matching the verified
/// `syn-2frame-inter-64x64` fixture's config: a single reusable uniform 1x1 tile (so
/// `tile_info()` reads no bits), restoration and CCSO disabled (the shared-tail admission
/// gate's verified subset), and `enable_df_sub_pu` on (so the inter deblocking arm reads
/// `allow_df_sub_pu`). Used to build COMPLETE inter headers in the focused tests below.
fn minimal_inter_seq_64() -> CoreSeqView {
    use crate::tile::TileParams;
    let mut seq = CoreSeqView::new_minimal_intra(64, 64).expect("64x64 is valid");
    seq.inter.explicit_ref_frame_map = false;
    seq.inter.enable_ref_frame_mvs = true;
    seq.filter.enable_df_sub_pu = true;
    seq.tile.seq_tile_info_present_flag = true;
    seq.tile.allow_tile_info_change = false;
    seq.tile.seq_tile_params = Some(TileParams {
        tile_cols: 1,
        tile_rows: 1,
        tile_cols_log2: 0,
        tile_rows_log2: 0,
        sb_cols: 1,
        sb_rows: 1,
        uniform_spacing: true,
        covers_cols: true,
        covers_rows: true,
    });
    seq
}

/// Builds the inter control-region prefix (activation + output flags + the implicit-map
/// reference control region) for `minimal_inter_seq_64`, up to and including
/// `disable_cdf_update` (the shared-tail boundary, mirror :5041). The caller appends the
/// shared tail. With one valid reference slot the implicit `get_ref_frames()` derives
/// `NumTotalRefs == 1`, `ref_frame_idx == [0]` (no bits).
fn minimal_inter_control_prefix(bits: &mut Bits) {
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(1); // frame_is_inter == 1
    bits.bit(1); // immediate_output_frame == 1 -> implicit_output_frame inferred 0 (no bit)
    bits.bit(0); // frame_size_override_flag
    bits.f(1, 4); // order_hint
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init
    bits.f(0, 8); // refresh_frame_flags f(NumRefFrames == 8)
    bits.bit(0); // use_ref_frame_mvs = 0
    bits.bit(0); // intrabc_params(): allow_intrabc = 0
    bits.bit(0); // use_qtr_precision_mv = 0
    bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
    bits.bit(1); // is_filter_switchable = 1
    bits.bit(0); // disable_cdf_update f(1), the shared-tail boundary.
}

fn minimal_inter_shared_tail_before_lr(bits: &mut Bits) {
    bits.f(90, 8); // quantization_params(): base_q_idx
    for _ in 0..6 {
        bits.bit(0); // segmentation/QM/delta-Q/deblocking flags
    }
}

/// The post-key reference state the fixture parse uses: only slot 0 valid (OrderHint 0,
/// 64x64), so the implicit map resolves to the single-reference case.
fn one_valid_ref_64() -> (
    [bool; NUM_REF_FRAMES],
    [u32; NUM_REF_FRAMES],
    [u32; NUM_REF_FRAMES],
    [u32; NUM_REF_FRAMES],
) {
    let mut ref_valid = [false; NUM_REF_FRAMES];
    let ref_oh = [0u32; NUM_REF_FRAMES];
    let mut ref_w = [0u32; NUM_REF_FRAMES];
    let mut ref_h = [0u32; NUM_REF_FRAMES];
    ref_valid[0] = true;
    ref_w[0] = 64;
    ref_h[0] = 64;
    (ref_valid, ref_oh, ref_w, ref_h)
}

#[test]
fn wrapped_display_order_hint_drives_implicit_reference_ranking() {
    let mut bits = Bits::default();
    minimal_inter_control_prefix(&mut bits);
    let data = bits.into_bytes();
    let seq = minimal_inter_seq_64();

    let mut ref_valid = [false; NUM_REF_FRAMES];
    ref_valid[0] = true;
    ref_valid[1] = true;
    let mut ref_order_hint = [0u32; NUM_REF_FRAMES];
    ref_order_hint[0] = 127;
    ref_order_hint[1] = 126;
    let ref_order_hint_lsbs = ref_order_hint;
    let ref_w = [64u32; NUM_REF_FRAMES];
    let ref_h = [64u32; NUM_REF_FRAMES];
    let ref_q = [40u32; NUM_REF_FRAMES];
    let ref_implicit = [false; NUM_REF_FRAMES];
    let ref_immediate = [true; NUM_REF_FRAMES];
    let reference_state = FrameReferenceStateView::from_slots_with_base_q_idx(
        &ref_valid,
        &ref_order_hint,
        &ref_w,
        &ref_h,
        &ref_q,
    )
    .with_single_layer_order_hint_state(&ref_order_hint_lsbs, &ref_implicit, &ref_immediate);

    let (core, _) = parse_body_with_ref(
        &data,
        ObuType::RegularTileGroup,
        false,
        &seq,
        None,
        &reference_state,
    )
    .unwrap();
    assert_eq!(core.order_hint_lsb, Some(1));
    assert_eq!(core.order_hint, Some(129));
    assert_eq!(core.inter.unwrap().ref_frame_idx, vec![0, 1]);
}

fn tip_output_seq_64() -> CoreSeqView {
    let mut seq = CoreSeqView::new_minimal_intra(64, 64).expect("64x64 is valid");
    seq.inter.explicit_ref_frame_map = true;
    seq.inter.enable_ref_frame_mvs = true;
    seq.inter.enable_tip = true;
    seq.inter.enable_tip_output = true;
    seq.inter.enable_tip_hole_fill = true;
    seq.inter.enable_tip_explicit_qp = true;
    seq.inter.enable_tip_refinemv = true;
    seq.inter.enable_opfl_refine = 1;
    seq.filter.enable_df_sub_pu = true;
    seq.film_grain_params_present = Some(true);
    seq
}

/// Builds an exactly byte-aligned TIP-as-output header through `tip_sharp`. The two
/// references straddle order hint 4, so equal weighting is inferred and no weight bits read.
fn tip_output_control_prefix(bits: &mut Bits) {
    bits.uvlc(0); // cur_mfh_id
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(1); // immediate_output_frame; implicit_output_frame inferred 0
    bits.bit(0); // frame_size_override_flag
    bits.f(4, 4); // order_hint
    bits.bit(0); // signal_primary_ref_frame
    bits.f(0, 8); // refresh_frame_flags
    bits.bit(1); // frame_explicit_ref_frame_map
    bits.f(2, 3); // num_total_refs
    bits.f(0, 3); // ref_frame_idx[0]
    bits.f(1, 3); // ref_frame_idx[1]
    bits.bit(1); // use_ref_frame_mvs
    bits.bit(0); // tmvp_sample_step_minus_1
    bits.bit(0); // allow_tip_hole_fill
    bits.bit(1); // tip_mv_zero
    bits.bit(1); // tip_sharp
}

fn two_tip_refs_64() -> (
    [bool; NUM_REF_FRAMES],
    [u32; NUM_REF_FRAMES],
    [u32; NUM_REF_FRAMES],
    [u32; NUM_REF_FRAMES],
) {
    let mut ref_valid = [false; NUM_REF_FRAMES];
    let mut ref_oh = [0u32; NUM_REF_FRAMES];
    let mut ref_w = [0u32; NUM_REF_FRAMES];
    let mut ref_h = [0u32; NUM_REF_FRAMES];
    ref_valid[0] = true;
    ref_valid[1] = true;
    ref_oh[0] = 2;
    ref_oh[1] = 6;
    ref_w[0] = 64;
    ref_w[1] = 64;
    ref_h[0] = 64;
    ref_h[1] = 64;
    (ref_valid, ref_oh, ref_w, ref_h)
}

#[test]
fn frame_header_core_tip_output_parses_terminal_tail() {
    let seq = tip_output_seq_64();
    let mut bits = Bits::default();
    tip_output_control_prefix(&mut bits);
    bits.f(77, 8); // explicit TIP base_q_idx
    bits.bit(1); // allow_df_sub_pu
    bits.bit(1); // apply_deblocking_filter_tip
    bits.bit(1); // tile_info(): uniform_tile_spacing_flag
    bits.bit(0); // film_grain_config(): apply_grain
    let data = bits.into_bytes();
    let (rv, roh, rw, rh) = two_tip_refs_64();
    let implicit = [false; NUM_REF_FRAMES];
    let immediate = [true; NUM_REF_FRAMES];
    let rs = FrameReferenceStateView::from_slots(&rv, &roh, &rw, &rh)
        .with_single_layer_order_hint_state(&roh, &implicit, &immediate);

    let (core, consumed) =
        parse_body_with_ref(&data, ObuType::RegularTip, false, &seq, None, &rs).unwrap();

    assert_eq!(core.status, FrameHeaderParseStatus::InterHeaderComplete);
    assert_eq!(core.quantization_params.as_ref().unwrap().base_q_idx, 77);
    let tile = core.tile_info.as_ref().unwrap();
    assert_eq!((tile.tile_cols, tile.tile_rows), (1, 1));
    let inter = core.inter.as_ref().unwrap();
    assert_eq!(inter.stop, Some(InterStop::TipAsOutputReturn));
    assert_eq!(inter.allow_df_sub_pu, Some(true));
    assert_eq!(inter.apply_deblocking_filter_tip, Some(true));
    assert!(!inter.tip_film_grain.as_ref().unwrap().apply_grain);
    assert_eq!(consumed, 44);
}

#[test]
fn frame_header_core_tip_output_infers_disabled_tail_gates() {
    let mut seq = tip_output_seq_64();
    seq.inter.enable_tip_explicit_qp = false;
    seq.filter.enable_df_sub_pu = false;
    seq.film_grain_params_present = Some(false);
    let mut bits = Bits::default();
    tip_output_control_prefix(&mut bits);
    let data = bits.into_bytes();
    let (rv, roh, rw, rh) = two_tip_refs_64();
    let implicit = [false; NUM_REF_FRAMES];
    let immediate = [true; NUM_REF_FRAMES];
    let rs = FrameReferenceStateView::from_slots(&rv, &roh, &rw, &rh)
        .with_single_layer_order_hint_state(&roh, &implicit, &immediate);

    let (core, consumed) =
        parse_body_with_ref(&data, ObuType::RegularTip, false, &seq, None, &rs).unwrap();

    assert_eq!(core.status, FrameHeaderParseStatus::InterHeaderComplete);
    assert!(core.quantization_params.is_none());
    assert!(core.tile_info.is_none());
    let inter = core.inter.as_ref().unwrap();
    assert_eq!(inter.allow_df_sub_pu, Some(false));
    assert_eq!(inter.apply_deblocking_filter_tip, Some(false));
    assert!(!inter.tip_film_grain.as_ref().unwrap().apply_grain);
    assert_eq!(consumed, 32);
}

#[test]
fn frame_header_core_tip_output_eof_in_tail_is_truncation() {
    let seq = tip_output_seq_64();
    let mut bits = Bits::default();
    tip_output_control_prefix(&mut bits);
    let data = bits.into_bytes();
    let (rv, roh, rw, rh) = two_tip_refs_64();
    let implicit = [false; NUM_REF_FRAMES];
    let immediate = [true; NUM_REF_FRAMES];
    let rs = FrameReferenceStateView::from_slots(&rv, &roh, &rw, &rh)
        .with_single_layer_order_hint_state(&roh, &implicit, &immediate);

    let (core, consumed) =
        parse_body_with_ref(&data, ObuType::RegularTip, false, &seq, None, &rs).unwrap();

    assert_eq!(
        core.status,
        FrameHeaderParseStatus::StoppedInsideInterControl
    );
    assert!(core.status.is_truncated_in_modeled_region());
    let inter = core.inter.as_ref().unwrap();
    assert_eq!(inter.stop, Some(InterStop::TipAsOutputReturn));
    assert_eq!(inter.allow_df_sub_pu, None);
    assert_eq!(inter.tip_film_grain, None);
    assert_eq!(consumed, 32);
}

#[test]
fn frame_header_core_inter_shared_tail_reads_inter_arms_with_asymmetric_values() {
    let mut seq = minimal_inter_seq_64();
    seq.film_grain_params_present = Some(true);
    let mut bits = Bits::default();
    minimal_inter_control_prefix(&mut bits);
    minimal_inter_shared_tail_before_lr(&mut bits);
    bits.bit(1); // read_tx_mode(): tx_mode_select = 1 -> TX_MODE_SELECT
    bits.bit(1); // frame_reference_mode(): reference_select = 1
    bits.bit(1); // skip_mode_params(): skip_mode_present = 1 (skipModeAllowed)
    bits.f(2, 2); // reduced_tx_set f(2) = 2
    bits.bit(1); // film_grain_config(): apply_grain = 1 (output frame, grain present)
    bits.f(5, 3); // fgm_id
    bits.f(0x1234, 16); // grain_seed
    let data = bits.into_bytes();
    let (rv, roh, rw, rh) = one_valid_ref_64();
    let rs = FrameReferenceStateView::from_slots(&rv, &roh, &rw, &rh);
    let (core, _) =
        parse_body_with_ref(&data, ObuType::RegularTileGroup, false, &seq, None, &rs).unwrap();
    assert_eq!(core.status, FrameHeaderParseStatus::InterHeaderComplete);
    let quant = core.quantization_params.as_ref().unwrap();
    assert_eq!(quant.base_q_idx, 90);
    let tail = core.inter_tail.as_ref().expect("inter tail parsed");
    assert_eq!(tail.tx_mode, TxMode::Select);
    assert!(
        tail.reference_select,
        "reference_select f(1) == 1 read distinctly"
    );
    assert!(
        tail.skip_mode_present,
        "skip_mode_present f(1) == 1 read distinctly"
    );
    assert!(!tail.allow_bawp);
    assert!(!tail.allow_warpmv_mode);
    assert_eq!(tail.reduced_tx_set, 2);
    assert!(!tail.global_motion.use_global_motion);
    assert!(tail.film_grain.apply_grain);
    assert_eq!(tail.film_grain.fgm_id, Some(5));
    assert_eq!(tail.film_grain.grain_seed, Some(0x1234));
}

#[test]
fn frame_header_core_inter_missing_lr_reference_taps_is_a_coverage_stop() {
    let mut seq = minimal_inter_seq_64();
    seq.restoration.enable_restoration = true;
    seq.restoration.lr_pc_wiener_disabled = true;
    seq.restoration.lr_uv_pc_wiener_disabled = true;
    let mut bits = Bits::default();
    minimal_inter_control_prefix(&mut bits);
    minimal_inter_shared_tail_before_lr(&mut bits);
    bits.ns(1, 2); // plane 0 -> RESTORE_WIENER_NONSEP
    bits.bit(1); // frame_filters_on[0]
    bits.bit(0); // temporal_pred_flag[0]
    bits.f(0, 3); // one local filter class
    bits.ns(0, 2); // plane 1 -> RESTORE_NONE
    bits.ns(0, 2); // plane 2 -> RESTORE_NONE
    bits.bit(1); // lr_luma_use_half_size
    bits.bit(1); // local class selects the retained reference filter
    let data = bits.into_bytes();
    let (rv, roh, rw, rh) = one_valid_ref_64();
    let filter_counts = [[1, 0, 0]; NUM_REF_FRAMES];
    let filter_taps: [crate::headers::frame::restoration::SlotFrameFilterTaps; NUM_REF_FRAMES] =
        std::array::from_fn(|_| None);
    let rs = FrameReferenceStateView::from_slots(&rv, &roh, &rw, &rh)
        .with_lr_frame_filter_class_counts(&filter_counts)
        .with_lr_frame_filter_taps(&filter_taps);
    let (core, _) =
        parse_body_with_ref(&data, ObuType::RegularTileGroup, false, &seq, None, &rs).unwrap();
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: "lr_temporal_reference_filter_match",
        }
    );
    assert!(!core.status.is_truncated_in_modeled_region());
    assert!(core.quantization_params.is_some());
    assert!(core.deblocking_filter_params.is_some());
    assert!(core.lr_params.is_none());
}

#[test]
fn frame_header_core_inter_mfh_segmentation_reuses_mfh_feature_data() {
    let seq = minimal_inter_seq_64();
    let mut mfh_features = [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS];
    mfh_features[3][0] = SegmentFeature {
        enabled: true,
        data: 7,
    };
    let mut record = mfh_record(
        None,
        Some(&(
            false,
            false,
            SegmentInfo {
                num_segments: 8,
                features: mfh_features,
            },
        )),
    );
    record.mfh_deblocking_filter_update = true;
    record.mfh_apply_deblocking_filter = [true, false, true, true];
    let view = MfhFrameView::from_record(&record, &seq);

    let mut bits = Bits::default();
    bits.uvlc(1); // cur_mfh_id == 1, resolved from the MFH view.
    bits.bit(1); // frame_is_inter == 1
    bits.bit(1); // immediate_output_frame == 1 -> implicit_output_frame inferred 0
    bits.bit(1); // frame_size_override_flag
    bits.f(1, 4); // order_hint
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init
    bits.f(0, 8); // refresh_frame_flags
    bits.bit(1); // frame_size_with_refs(): found_ref = 1 -> slot 0 dimensions
    bits.bit(0); // use_ref_frame_mvs = 0
    bits.bit(0); // intrabc_params(): allow_intrabc = 0
    bits.bit(0); // use_qtr_precision_mv = 0
    bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
    bits.bit(1); // is_filter_switchable = 1
    bits.bit(0); // disable_cdf_update
    bits.f(90, 8); // quantization_params(): base_q_idx
    bits.bit(1); // segmentation_enabled; MFH reuse consumes no seg_info bits
    bits.bit(0); // setup_qm_params(): using_qmatrix = 0
    bits.bit(0); // delta_q_params(): delta_q_present = 0
    bits.bit(0); // allow_df_sub_pu
    bits.bit(0); // df_delta_q_present[0] from MFH apply_deblocking_filter[0]
    bits.bit(0); // df_delta_q_present[2] from MFH apply_deblocking_filter[2]
    bits.bit(0); // df_delta_q_present[3] from MFH apply_deblocking_filter[3]
    bits.bit(1); // read_tx_mode(): tx_mode_select = 1 -> TX_MODE_SELECT
    bits.bit(1); // frame_reference_mode(): reference_select = 1
    bits.bit(1); // skip_mode_params(): skip_mode_present = 1
    bits.f(2, 2); // reduced_tx_set
    bits.bit(0); // film_grain_config(): apply_grain = 0
    let data = bits.into_bytes();

    let (rv, roh, rw, rh) = one_valid_ref_64();
    let ref_q = [90u32; NUM_REF_FRAMES];
    let ref_counter = [0u32; NUM_REF_FRAMES];
    let ref_is_inter = [false; NUM_REF_FRAMES];
    let rs = FrameReferenceStateView::from_slots_with_base_q_idx(&rv, &roh, &rw, &rh, &ref_q)
        .with_primary_reference_state(&ref_counter, &ref_is_inter);
    let (core, _) = parse_body_with_ref(
        &data,
        ObuType::RegularTileGroup,
        false,
        &seq,
        Some(&view),
        &rs,
    )
    .unwrap();

    assert_eq!(core.status, FrameHeaderParseStatus::InterHeaderComplete);
    let seg = core.segmentation_params.expect("inter segmentation parsed");
    assert!(seg.segmentation_enabled);
    assert!(seg.segmentation_update_map);
    assert!(!seg.segmentation_temporal_update);
    assert!(seg.features[3][0].enabled);
    assert_eq!(seg.features[3][0].data, 7);
    let deblocking = core
        .deblocking_filter_params
        .expect("inter deblocking parsed");
    assert_eq!(
        deblocking.apply_deblocking_filter,
        [true, false, true, true]
    );
    assert!(core.inter_tail.is_some());
}

#[test]
fn frame_header_core_inter_missing_mfh_stops_before_mfh_gated_tail() {
    let seq = minimal_inter_seq_64();
    let mut bits = Bits::default();
    bits.uvlc(1); // cur_mfh_id == 1, but no MFH view is supplied.
    bits.bit(1); // frame_is_inter == 1
    bits.bit(1); // immediate_output_frame == 1 -> implicit_output_frame inferred 0
    bits.bit(1); // frame_size_override_flag
    bits.f(1, 4); // order_hint
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init
    bits.f(0, 8); // refresh_frame_flags
    bits.bit(1); // frame_size_with_refs(): found_ref = 1 -> slot 0 dimensions
    bits.bit(0); // use_ref_frame_mvs = 0
    bits.bit(0); // intrabc_params(): allow_intrabc = 0
    bits.bit(0); // use_qtr_precision_mv = 0
    bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
    bits.bit(1); // is_filter_switchable = 1
    bits.bit(0); // disable_cdf_update
    bits.f(90, 8); // quantization_params(): base_q_idx
    bits.bit(1); // padding that must not be read as segmentation_enabled
    let data = bits.into_bytes();

    let (rv, roh, rw, rh) = one_valid_ref_64();
    let rs = FrameReferenceStateView::from_slots(&rv, &roh, &rw, &rh);
    let (core, _) =
        parse_body_with_ref(&data, ObuType::RegularTileGroup, false, &seq, None, &rs).unwrap();

    assert!(matches!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
    ));
    assert_eq!(
        core.inter.as_ref().unwrap().stop,
        Some(crate::headers::frame::inter::InterStop::ReachedSharedTail)
    );
    assert!(core.segmentation_params.is_none());
    assert!(core.deblocking_filter_params.is_none());
    assert!(core.inter_tail.is_none());
}

#[test]
fn frame_header_core_inter_shared_tail_segmentation_on_stops_unmodeled() {
    let seq = minimal_inter_seq_64();
    let mut bits = Bits::default();
    minimal_inter_control_prefix(&mut bits);
    bits.f(90, 8); // base_q_idx
    bits.bit(1); // segmentation_enabled = 1 -> honest stop right after this bit
    bits.f(0, 8); // padding (never read)
    let data = bits.into_bytes();
    let (rv, roh, rw, rh) = one_valid_ref_64();
    let rs = FrameReferenceStateView::from_slots(&rv, &roh, &rw, &rh);
    let (core, _) =
        parse_body_with_ref(&data, ObuType::RegularTileGroup, false, &seq, None, &rs).unwrap();
    assert!(matches!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
    ));
    assert!(!core.status.is_truncated_in_modeled_region());
    assert_eq!(
        core.inter.as_ref().unwrap().stop,
        Some(crate::headers::frame::inter::InterStop::ReachedSharedTail)
    );
    assert!(core.inter_tail.is_none());
}

#[test]
fn frame_header_core_inter_shared_tail_ccso_on_reaches_complete_tail() {
    let mut seq = minimal_inter_seq_64();
    seq.ccso.enable_ccso = true;
    let mut bits = Bits::default();
    minimal_inter_control_prefix(&mut bits);
    bits.f(0, 16);
    let data = bits.into_bytes();
    let (rv, roh, rw, rh) = one_valid_ref_64();
    let rs = FrameReferenceStateView::from_slots(&rv, &roh, &rw, &rh);
    let (core, _) =
        parse_body_with_ref(&data, ObuType::RegularTileGroup, false, &seq, None, &rs).unwrap();
    assert_eq!(core.status, FrameHeaderParseStatus::InterHeaderComplete);
    assert!(core.quantization_params.is_some());
    assert!(core.setup_qm_params.is_some());
    assert!(core.ccso_params.is_some());
    assert!(core.inter_tail.is_some());
    assert_eq!(
        core.inter.as_ref().unwrap().stop,
        Some(crate::headers::frame::inter::InterStop::ReachedSharedTail)
    );
}

#[test]
fn frame_header_core_inter_eof_inside_control_region_is_truncation() {
    let mut seq = base_seq();
    seq.inter.explicit_ref_frame_map = true;
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(1); // frame_is_inter == 1
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(0); // frame_size_override_flag
    bits.f(7, 4); // order_hint
    bits.bit(0); // signal_primary_ref_frame
    bits.bit(0); // disable_cross_frame_cdf_init
    bits.f(0, 8); // refresh_frame_flags
    bits.bit(1); // frame_explicit_ref_frame_map
    bits.f(2, 3); // num_total_refs = 2 (last field that fits; ref_frame_idx truncated)
    let data = bits.into_bytes();
    assert_eq!(
        data.len(),
        3,
        "the test relies on an exact 3-byte truncation"
    );
    let (core, _) = parse_body(&data, ObuType::RegularTileGroup, true, &seq).unwrap();

    assert_eq!(
        core.status,
        FrameHeaderParseStatus::StoppedInsideInterControl,
        "an EOF inside the modeled inter control region is a truncation status"
    );
    assert!(
        core.status.is_truncated_in_modeled_region(),
        "StoppedInsideInterControl is on the truncated-in-modeled-region side"
    );
    assert_eq!(core.frame_type, Some(FrameType::Inter));
    assert_eq!(core.immediate_output_frame, Some(false));
    assert_eq!(core.order_hint_lsb, Some(7));
    let inter = core.inter.as_ref().expect("partial inter facts preserved");
    assert_eq!(inter.explicit_ref_frame_map, Some(true));
    assert_eq!(
        inter.num_total_refs,
        Some(2),
        "num_total_refs (the last field read before the EOF) is preserved"
    );
    assert!(
        inter.ref_frame_idx.is_empty(),
        "ref_frame_idx was being read when the payload ran out"
    );
}

#[test]
fn frame_header_core_bridge_eof_inside_control_region_is_truncation() {
    let mut bits = Bits::default();
    bits.uvlc(0); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
    bits.f(5, 3); // bridge_frame_ref_idx = 5 f(CeilLog2(8) == 3)
    bits.bit(0); // bridge_frame_overwrite_flag = 0 -> refresh = 1 << 5 (no bits)
    bits.f(0b1111, 4); // only 4 of the 12 width bits, then EOF
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::BridgeFrame, true, &base_seq()).unwrap();

    assert_eq!(
        core.status,
        FrameHeaderParseStatus::StoppedInsideInterControl,
        "an EOF inside the modeled bridge control region is a truncation status"
    );
    assert!(core.status.is_truncated_in_modeled_region());
    assert!(core.is_bridge);
    assert_eq!(core.bridge_frame_ref_idx, Some(5));
    let inter = core.inter.as_ref().expect("partial bridge facts preserved");
    assert_eq!(
        inter.bridge_frame_overwrite_flag,
        Some(false),
        "the bridge_frame_overwrite_flag read before the EOF is preserved"
    );
    assert_eq!(inter.refresh_frame_flags, Some(1 << 5));
}

#[test]
fn frame_header_core_ras_with_unknown_reference_state_stops_after_long_term_ids() {
    let mut seq = base_seq();
    seq.long_term_frame_id_bits = 4;
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(0); // restricted_prediction_switch
    bits.f(2, 3); // num_key_ref_frames == 2
    bits.f(5, 4); // ref_long_term_id[0]
    bits.f(9, 4); // ref_long_term_id[1]
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.f(3, 4); // order_hint f(OrderHintBits == 4)
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::RasFrame, true, &seq).unwrap();

    assert_eq!(core.frame_type, Some(FrameType::Switch));
    assert_eq!(core.frame_is_intra, Some(false));
    assert_eq!(core.order_hint_lsb, Some(3));
    assert_eq!(core.frame_size_override_flag, Some(true));
    assert!(matches!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
    ));
    let inter = core.inter.as_ref().unwrap();
    assert_eq!(
        inter.stop,
        Some(crate::headers::frame::inter::InterStop::UnmodeledDerivation)
    );
    assert!(!core.forbidden_ref_long_term_id);
}

#[test]
fn frame_header_core_flags_reserved_ref_long_term_id() {
    let mut seq = base_seq();
    seq.long_term_frame_id_bits = 4;
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(0); // restricted_prediction_switch
    bits.f(1, 3); // num_key_ref_frames == 1
    bits.f(15, 4); // ref_long_term_id[0] == (1 << 4) - 1 (reserved)
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::RasFrame, true, &seq).unwrap();
    assert!(core.forbidden_ref_long_term_id);
}

#[test]
fn frame_header_core_eof_in_ref_long_term_id_loop() {
    let mut seq = base_seq();
    seq.long_term_frame_id_bits = 4;
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(0); // restricted_prediction_switch
    bits.f(7, 3); // num_key_ref_frames == 7; the ref_long_term_id loop overruns
    let data = bits.into_bytes();
    let err = parse_body(&data, ObuType::RasFrame, true, &seq).unwrap_err();
    assert!(matches!(err, Error::UnexpectedEof { .. }));
}

#[test]
fn frame_header_core_olk_reads_long_term_ids_then_intra_tail() {
    let mut seq = base_seq();
    seq.long_term_frame_id_bits = 4;
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(1, 4); // long_term_id_plus_1
    bits.f(1, 3); // num_key_ref_frames == 1
    bits.f(3, 4); // ref_long_term_id[0]
    bits.bit(0); // implicit_output_frame
    bits.bit(0); // frame_size_override_flag (cur_mfh_id == 0 -> max dims)
    bits.f(2, 4); // order_hint
    bits.f(0b0000_0101, 8); // refresh_frame_flags f(NumRefFrames == 8)
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    bits.bit(1); // uniform_tile_spacing_flag
    bits.bit(0); // increment_tile_cols_log2 = 0
    bits.bit(0); // increment_tile_rows_log2 = 0
    bits.f(45, 8); // base_q_idx
    bits.bit(0); // segmentation_enabled
    bits.bit(0); // using_qmatrix
    bits.bit(0); // delta_q_present
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    bits.bit(0); // tx_mode_select = 0
    bits.f(0, 2); // reduced_tx_set = 0
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::OpenLoopKey, true, &seq).unwrap();

    assert_eq!(core.frame_type, Some(FrameType::Key));
    assert_eq!(core.frame_is_intra, Some(true));
    assert_eq!(core.immediate_output_frame, Some(false));
    assert_eq!(core.implicit_output_frame, Some(false));
    assert_eq!(core.order_hint_lsb, Some(2));
    assert_eq!(core.refresh_frame_flags, Some(0b0000_0101));
    assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
    assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
    assert!(core.intra_tail.is_some());
}

#[test]
fn display_order_hint_extends_past_the_coded_lsb_window() {
    let valid = [true];
    let hints = [118];
    let lsbs = [118];
    let implicit = [false];
    let immediate = [true];
    let widths = [64];
    let heights = [64];
    let reference_state = FrameReferenceStateView::from_slots(&valid, &hints, &widths, &heights)
        .with_single_layer_order_hint_state(&lsbs, &implicit, &immediate);

    assert_eq!(
        get_disp_order_hint(
            ObuType::RegularTileGroup,
            Some(FrameType::Inter),
            None,
            8,
            7,
            &reference_state,
        ),
        Some(136)
    );
}

#[test]
fn display_order_hint_ignores_non_showable_references() {
    let valid = [true];
    let hints = [118];
    let lsbs = [118];
    let output = [false];
    let widths = [64];
    let heights = [64];
    let reference_state = FrameReferenceStateView::from_slots(&valid, &hints, &widths, &heights)
        .with_single_layer_order_hint_state(&lsbs, &output, &output);

    assert_eq!(
        get_disp_order_hint(
            ObuType::RegularTileGroup,
            Some(FrameType::Inter),
            None,
            8,
            7,
            &reference_state,
        ),
        Some(8)
    );
}

mod tail;
