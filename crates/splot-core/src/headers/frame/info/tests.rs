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
    // The representative non-single-picture intra sequence view is exactly the
    // public encoder writer-input constructor, so the whole frame-header round-trip
    // suite regresses `CoreSeqView::new_minimal_intra`.
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
    // CLK, cur_mfh_id == 0, seq_header_id_in_frame_header == 1, full intra path.
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(1); // seq_header_id_in_frame_header
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(1); // frame_size_override_flag
    bits.f(5, 4); // order_hint
    // refresh_frame_flags: CLK + max_mlayer_id == 0 -> allFrames (no bits)
    bits.f(1920 - 1, 12); // frame_width_minus_1
    bits.f(1080 - 1, 12); // frame_height_minus_1
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    // tile_info() (§ 5.18.7.2): no sequence tile info -> tile_params(). 1920x1080
    // with 128x128 superblocks: sbCols = 15, sbRows = 9, single uniform tile.
    bits.bit(1); // uniform_tile_spacing_flag
    bits.bit(0); // increment_tile_cols_log2 = 0
    bits.bit(0); // increment_tile_rows_log2 = 0
    // quantization_params() (§ 5.18.6.1): 8-bit -> base_q_idx f(8); all delta
    // reads disabled in the test view.
    bits.f(90, 8); // base_q_idx
    bits.bit(0); // segmentation_enabled (§ 5.18.7.1)
    bits.bit(0); // using_qmatrix (§ 5.18.6.2)
    bits.bit(0); // delta_q_present (§ 5.18.7.8, base_q_idx > 0)
    // § 5.18.2 lossless tail: base_q_idx 90 -> CodedLossless = 0; allow_tcq is
    // inferred enable_tcq (0) and allow_parity_hiding is forced 0 (no bits).
    // deblocking_filter_params() (§ 5.18.5.2): not lossless -> apply[0]/[1] read.
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1] (both 0 -> no chroma pair, no delta-Q)
    // gdf_params() / cdef_params(): enable_gdf == enable_cdef == 0 -> no bits.
    // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
    // § 5.18.2 tail: read_tx_mode() not lossless -> tx_mode_select f(1); reduced_tx_set
    // f(2); film_grain_config() grain absent -> apply_grain inferred 0, no bits.
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
    // uvlc(0)=1 + uvlc(1)=3 prefix bits, then 33 core bits (1+1+1+4 control/output,
    // 24 frame_size, 1 allow_intrabc, 1 disable_cdf_update), then 14 structure
    // bits (3 tile_info, 8 base_q_idx, 1 segmentation_enabled, 1 using_qmatrix,
    // 1 delta_q_present), then 2 deblocking apply bits (GDF/CDEF disabled -> 0 bits),
    // then 3 tail bits (tx_mode_select + reduced_tx_set; grain absent).
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
    // cur_mfh_id == 1, resolved MFH with mfh_deblocking_filter_update == 1 and
    // mfh_apply_deblocking_filter == [1, 0, 1, 1]: § 5.18.5.2 copies apply from the
    // MFH (no apply bits read), and NumPlanes == 3 with apply[0] set copies the
    // chroma pair. Only the per-i df_delta_q_present bits are read.
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
    // deblocking_filter_params(): MFH arm -> apply copied [1,0,1,1], no apply bits.
    // df_delta_q_present read for i in {0, 2, 3} (apply set); i == 1 skipped.
    bits.bit(0); // df_delta_q_present[0]
    bits.bit(0); // df_delta_q_present[2]
    bits.bit(0); // df_delta_q_present[3]
    // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
    // § 5.18.2 tail: tx_mode_select f(1) + reduced_tx_set f(2); grain absent.
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
    // tile_info() for a 16x16 frame with 128x128 superblocks is a single superblock
    // (MiCols == MiRows == 4, sbCols == sbRows == 1), so tile_params() reads only
    // uniform_tile_spacing_flag and skips the increment / context fields.
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
    // REGRESSION (codex F2): a payload truncated mid-deblocking_filter_params() must
    // NOT fail the whole core parse. Before the loop-filter cluster existed the parser
    // stopped here and returned Ok with the control-region facts; the validator/inspect
    // .ok() the result, so an Err would silently drop every earlier state-supported
    // diagnostic. The parser now keeps the facts and reports StoppedInsideFilterParams.
    //
    // byte_aligned_filter_seq() puts the loop-filter cluster on byte 6 (bit 48), so
    // deblocking's first read (apply_deblocking_filter[0]) sits in byte 6. Truncating
    // the payload to 6 bytes makes that read overrun, landing the EOF at the very start
    // of the cluster with deblocking_filter_params still None.
    let mut bits = intra_body_up_to_filter_cluster();
    let cluster_start = bits.bit_len();
    assert_eq!(
        cluster_start, 48,
        "with order_hint_bits == 5 the loop-filter cluster starts on byte 6"
    );
    // deblocking apply[0] is the cluster's first read (bit 48 = byte 6). Truncating to
    // 6 bytes makes that read overrun, landing the EOF at the very start of the cluster.
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
    // The payload parses cleanly through deblocking_filter_params() and into
    // gdf_params(), then runs out: deblocking is preserved, gdf/cdef stay None, and the
    // status is the truncation marker. deblocking is built to consume exactly the full
    // byte 6 (apply[0..4] = 1 + df_delta_q_present[0..4] = 0, 8 bits), so gdf begins at
    // the byte-7 boundary (bit 56) and truncating to 7 bytes drops the byte gdf needs.
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
    // The payload parses cleanly through deblocking and gdf (gdf disabled-by-flag so it
    // short-circuits with no reads) and into cdef_params(), then runs out: deblocking
    // and gdf are preserved, cdef stays None, status is the marker. deblocking again
    // consumes exactly byte 6 (8 bits) so cdef begins at the byte-7 boundary (bit 56).
    let mut seq = byte_aligned_filter_seq();
    seq.filter.enable_cdef = true; // cdef_params() reads bits instead of short-circuiting
    // enable_gdf stays false so gdf_params() short-circuits with no reads.
    let mut bits = intra_body_up_to_filter_cluster();
    bits.bit(1); // apply_deblocking_filter[0]
    bits.bit(1); // apply_deblocking_filter[1]
    bits.bit(1); // apply_deblocking_filter[2]
    bits.bit(1); // apply_deblocking_filter[3]
    bits.bit(0); // df_delta_q_present[0]
    bits.bit(0); // df_delta_q_present[1]
    bits.bit(0); // df_delta_q_present[2]
    bits.bit(0); // df_delta_q_present[3] -> deblocking ends at bit 56 (byte boundary)
    // gdf_params(): enable_gdf == false -> no bits.
    bits.bit(1); // cdef_frame_enable (byte 7) -> dropped
    let mut data = bits.into_bytes();
    data.truncate(7); // 56 bits: the cdef_frame_enable read overruns
    let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
    assert_truncated_filter_cluster_preserves_facts(&core);
    assert!(
        core.deblocking_filter_params.is_some(),
        "deblocking parsed before the cdef truncation must survive"
    );
    // gdf was frame-disabled, so its field is Some with gdf_frame_enable == false.
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
    // Restoration AND CCSO enabled: the intra tail parses cdef, then lr_params()
    // (no plane signals frame_filters_on, so no read_wienerns_filter) and ccso_params(),
    // then the §5.18.2 tail (read_tx_mode + reduced_tx_set, grain absent) to the
    // IntraHeaderComplete terminal. CDEF/GDF stay disabled so the cluster's only pre-lr
    // reads are the 2 deblocking apply bits.
    let mut seq = byte_aligned_filter_seq();
    // lr_tools both luma tools enabled; chroma PC-Wiener inferred disabled.
    seq.restoration.enable_restoration = true;
    seq.restoration.lr_uv_pc_wiener_disabled = true;
    seq.ccso.enable_ccso = true;
    let mut bits = intra_body_up_to_filter_cluster();
    // deblocking_filter_params(): not lossless -> apply[0]/[1] read, both 0.
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    // gdf_params() / cdef_params(): disabled -> no bits.
    // lr_params(): luma tool_index ns(4) == 0 -> RESTORE_NONE; chroma planes ns(2) == 0
    // -> RESTORE_NONE. No frame_filters_on, no size flags.
    bits.ns(0, 4); // plane 0 tool_index -> RESTORE_NONE
    bits.ns(0, 2); // plane 1 tool_index -> RESTORE_NONE
    bits.ns(0, 2); // plane 2 tool_index -> RESTORE_NONE
    // ccso_params(): not single picture -> ccso_frame_flag f(1) == 1, then all planes
    // ccso_planes == 0.
    bits.bit(1); // ccso_frame_flag
    bits.bit(0); // ccso_planes[0]
    bits.bit(0); // ccso_planes[1]
    bits.bit(0); // ccso_planes[2]
    // §5.18.2 tail: read_tx_mode() not lossless -> tx_mode_select f(1); reduced_tx_set
    // f(2); film_grain_config() grain absent (test_seq has film_grain_params_present ==
    // false) -> apply_grain inferred 0, no bits.
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
    // A luma plane selects RESTORE_WIENER_NONSEP and signals frame_filters_on. The
    // frame-level read_wienerns_filter(0, 0, 0, 1) path parses a two-class merged bank,
    // then ccso_params() and the intra tail complete.
    let mut seq = byte_aligned_filter_seq();
    seq.restoration.enable_restoration = true;
    seq.restoration.lr_pc_wiener_disabled = true;
    seq.restoration.lr_uv_pc_wiener_disabled = true;
    seq.ccso.enable_ccso = true;
    let mut bits = intra_body_up_to_filter_cluster();
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    // lr_params(): luma PC-Wiener disabled, so tool_index ns(2) == 1 selects
    // RESTORE_WIENER_NONSEP.
    bits.ns(1, 2); // plane 0 -> RESTORE_WIENER_NONSEP
    bits.bit(1); // frame_filters_on[0] == 1
    bits.f(1, 3); // num_filter_classes_idx == 1 -> Decode_Num_Filter_Classes[1] == 2
    bits.ns(0, 2); // plane 1 -> RESTORE_NONE
    bits.ns(0, 2); // plane 2 -> RESTORE_NONE
    bits.bit(1); // lr_luma_use_half_size
    // read_wienerns_filter(0, 0, 0, 1): class 1 matches prior class 0; both merged.
    bits.bit(0); // class 1 match_index == 1
    bits.bit(1); // merged[0]
    bits.bit(1); // merged[1]
    // ccso_params(): not single picture -> ccso_frame_flag f(1) == 1, then all planes
    // ccso_planes == 0.
    bits.bit(1); // ccso_frame_flag
    bits.bit(0); // ccso_planes[0]
    bits.bit(0); // ccso_planes[1]
    bits.bit(0); // ccso_planes[2]
    // §5.18.2 tail.
    bits.bit(1); // tx_mode_select = 1 -> TX_MODE_SELECT
    bits.f(3, 2); // reduced_tx_set = 3
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
    assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
    assert_eq!(core.frame_size, Some(FrameSize::new(16, 16)));
    assert!(core.deblocking_filter_params.is_some());
    assert_eq!(core.lr_params_partial, None);
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
    // The payload parses through deblocking and lr_params() (restoration disabled so it
    // reads nothing) and into ccso_params(), then runs out at the ccso_frame_flag read:
    // the earlier facts survive, ccso stays None, status is the truncation marker. The
    // deblocking reads consume exactly byte 6 (bit 56) so ccso begins at the byte-7
    // boundary.
    let mut seq = byte_aligned_filter_seq();
    // restoration disabled (lr reads nothing); ccso enabled (reads the frame flag).
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
    // gdf/cdef disabled -> no bits. lr disabled -> no bits.
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
    // The payload parses cleanly through ccso_params() but ends inside the § 5.18.2
    // tail (the reduced_tx_set f(2) read overruns): the control-region and loop-filter
    // facts survive, intra_tail stays None, and the status is StoppedInsideIntraTail.
    let mut seq = byte_aligned_filter_seq();
    seq.ccso.enable_ccso = true;
    let mut bits = intra_body_up_to_filter_cluster();
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    // gdf/cdef/lr disabled -> no bits. ccso enabled -> ccso_frame_flag f(1) + 3 planes.
    bits.bit(1); // ccso_frame_flag
    bits.bit(0); // ccso_planes[0]
    bits.bit(0); // ccso_planes[1]
    bits.bit(0); // ccso_planes[2]
    // § 5.18.2 tail: tx_mode_select f(1), then reduced_tx_set f(2) — supply only the
    // tx bit and ONE of the two reduced_tx_set bits, then truncate so the second
    // reduced_tx_set bit overruns.
    bits.bit(0); // tx_mode_select = 0 -> Largest
    bits.bit(0); // 1 of 2 reduced_tx_set bits; the next bit is missing
    let total_bits = bits.bit_len();
    let mut data = bits.into_bytes();
    // Truncate to the last whole byte that still contains the tx + partial reduced bits
    // but not a full second reduced_tx_set bit. total_bits here is mid-byte, so keeping
    // ceil(total_bits/8) - 0 bytes and not padding more leaves the read short.
    let keep_bytes = total_bits / 8; // drop the partial trailing byte -> reduced_tx_set overruns
    data.truncate(keep_bytes);
    let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
    assert_eq!(core.status, FrameHeaderParseStatus::StoppedInsideIntraTail);
    // Control-region and cluster facts survive; the tail itself was not committed.
    assert_eq!(core.frame_size, Some(FrameSize::new(16, 16)));
    assert!(core.deblocking_filter_params.is_some());
    assert!(core.lr_params.is_some());
    assert!(core.ccso_params.is_some());
    assert_eq!(core.intra_tail, None, "the truncated tail stays None");
}

#[test]
fn frame_header_core_intra_tail_with_grain_present_reads_id_and_seed() {
    // film_grain_params_present == true on an OUTPUT key frame (immediate_output_frame
    // == 1): film_grain_config()'s output gate is false, so apply_grain is read f(1);
    // when set, fgm_id f(3) + grain_seed f(16). Build the body with the output flag set
    // (the byte-aligned helper hardcodes both output flags to 0, which would force
    // apply_grain = 0).
    let mut seq = base_seq();
    seq.film_grain_params_present = Some(true);
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(1); // immediate_output_frame == 1 (output frame -> apply_grain readable)
    // implicit_output_frame inferred 0 (immediate_output_frame == 1), no bit.
    bits.bit(0); // frame_size_override_flag == 0 (cur_mfh_id == 0 -> max dims)
    bits.f(3, 4); // order_hint f(4)
    // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
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
    // gdf/cdef/lr/ccso all disabled in base_seq -> no bits.
    // § 5.18.2 tail: tx_mode_select f(1); reduced_tx_set f(2); film_grain_config()
    // grain present + immediate_output -> apply_grain f(1) + fgm_id f(3) + grain_seed f(16).
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
    // film_grain_params_present == None models an active sequence header recorded from a
    // bounded sequence_tile_config() stop (the flag is read last in § 5.4.1, after every
    // child config). Pre-fix CoreSeqView::from_sequence's `?` on the flag collapsed the
    // whole view, so the frame parse stopped at ActivationFieldsOnly and every
    // frame-size / output / order-hint diagnostic was suppressed. Now the control region
    // (which never consumes the flag) parses to completion and the parser stops honestly
    // at the film_grain_config() boundary — facts preserved, NOT a guessed apply_grain.
    let mut seq = base_seq();
    seq.film_grain_params_present = None;
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.bit(1); // immediate_output_frame == 1
    bits.bit(0); // frame_size_override_flag == 0 (cur_mfh_id == 0 -> max dims 4096x2304)
    bits.f(3, 4); // order_hint f(4)
    // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
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
    // gdf/cdef/lr/ccso disabled in base_seq -> no bits. The next structure is the
    // § 5.18.2 tail, whose film_grain_config() needs the (unknown) grain flag.
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
    // The control region parsed: these facts feed the validator's §6.17 diagnostics.
    assert_eq!(core.immediate_output_frame, Some(true));
    assert_eq!(core.order_hint_lsb, Some(3));
    assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
    assert!(core.quantization_params.is_some());
    assert!(core.deblocking_filter_params.is_some());
    // The parser stopped honestly at the film_grain_config() boundary, not at the prefix
    // (ActivationFieldsOnly) and never guessing apply_grain.
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
    // SEF whose active sequence header is a bounded stop (film_grain_params_present ==
    // None). The SEF fields (frame_to_show_map_idx, order hint, output flags) are parsed
    // and preserved, but film_grain_config() needs the unknown grain flag, so the parser
    // stops honestly rather than guessing apply_grain. Pre-fix the whole parse collapsed
    // to ActivationFieldsOnly.
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
    // SEF with film_grain_params_present == true: immediate_output_frame == 1 makes the
    // output gate false, so apply_grain is read f(1); when set, fgm_id + grain_seed.
    let mut seq = base_seq();
    seq.film_grain_params_present = Some(true);
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(6, 3); // frame_to_show_map_idx
    bits.bit(1); // derive_sef_order_hint == 1 -> no sef_order_hint
    // film_grain_config(): grain present, immediate_output -> apply_grain f(1).
    bits.bit(1); // apply_grain = 1
    bits.f(2, 3); // fgm_id = 2
    bits.f(0x1357, 16); // grain_seed
    // § 5.2.3 trailing_bits(): trailing_one_bit == 1, then into_bytes() zero-pads the
    // rest of the byte (the trailing_zero_bits) — a conformant SEF tail.
    bits.bit(1); // trailing_one_bit
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::RegularSef, true, &seq).unwrap();
    assert_eq!(core.show_existing_frame, Some(true));
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::ShowExistingFrameComplete
    );
    let fg = core.sef_film_grain.expect("SEF film_grain_config parsed");
    assert!(fg.apply_grain);
    assert_eq!(fg.fgm_id, Some(2));
    assert_eq!(fg.grain_seed, Some(0x1357));
    // A conformant SEF tail classifies as Valid (no diagnostic).
    assert_eq!(core.sef_trailing_bits, Some(SefTrailingBits::Valid));
}

#[test]
fn frame_header_core_sef_eof_inside_film_grain_preserves_facts() {
    // SEF with grain present but the payload ends inside film_grain_config(): the SEF
    // facts survive and the status reports StoppedInsideShowExistingFrame — the SEF
    // tail IS film_grain_config(), a fully-modeled region, so an EOF there is a
    // decidable truncation (distinct from the ordinary bounded CoreFieldsOnly stop),
    // surfaced as truncated-in-modeled-region. Not a hard error.
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
    // No trailing-bits boundary is recorded on a truncated SEF: the payload ended
    // inside film_grain_config(), so there is no completed tail to classify.
    assert_eq!(core.sef_trailing_bits, None);
}

#[test]
fn frame_header_core_sef_nonzero_bits_after_fields_flag_trailing_bits() {
    // A grain-free SEF whose payload carries arbitrary nonzero bits after the parsed
    // fields where § 5.2.3 trailing_bits() must be. Pre-fix this completed silently
    // (ShowExistingFrameComplete with no trailing-bits boundary); now the SEF tail is
    // classified and a non-conformant tail is recorded for the validator.
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(6, 3); // frame_to_show_map_idx
    bits.bit(1); // derive_sef_order_hint == 1 -> no sef_order_hint
    // No grain (base_seq has film_grain_params_present == false) -> apply_grain = 0.
    // The next bit must be the trailing_one_bit == 1; instead a 0 then arbitrary bits.
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
    // A SEF with grain where grain_seed is short by its final bit: the f(16) read
    // consumes what should have been the § 5.2.3 trailing_one_bit, leaving no marker.
    // Pre-fix this completed clean with a corrupted seed; now the trailing-bits check
    // fails (the marker was eaten), so the SEF no longer completes silently.
    let mut seq = base_seq();
    seq.film_grain_params_present = Some(true);
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(6, 3); // frame_to_show_map_idx
    bits.bit(1); // derive_sef_order_hint == 1
    bits.bit(1); // apply_grain = 1
    bits.f(2, 3); // fgm_id = 2
    // A conformant frame would code grain_seed f(16) then a trailing_one_bit. Here the
    // encoder coded only 15 distinct seed bits plus the marker bit, so the f(16) read
    // swallows the marker: 15 seed bits then the would-be trailing_one_bit as bit 16,
    // and into_bytes() zero-fills the rest — no trailing_one_bit remains.
    bits.f(0x0000, 15); // 15 seed bits
    bits.bit(1); // the marker bit, consumed as the 16th grain_seed bit
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::RegularSef, true, &seq).unwrap();
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::ShowExistingFrameComplete
    );
    let fg = core.sef_film_grain.expect("SEF film_grain_config parsed");
    // The seed is parsed (with the marker bit folded into it), but the trailing-bits
    // boundary is now non-conformant: the bytes after grain_seed are all zero, so the
    // first remaining bit is 0 (MissingOneBit) — or, if grain_seed ended exactly at a
    // byte boundary, no bits remain (Empty). Either is a recorded violation.
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
    // CLK, cur_mfh_id == 2 with NO resolved MFH record and
    // frame_size_override_flag == 0: the default dims come from the (unresolvable)
    // MFH, so the size is unknown and the parser stops before tile_info() without
    // guessing — the Unknown-routing case.
    let mut bits = Bits::default();
    bits.uvlc(2); // cur_mfh_id == 2 -> no seq_header_id_in_frame_header
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(0); // frame_size_override_flag == 0 (default dims)
    bits.f(7, 4); // order_hint
    // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
    // frame_size(): default path, no bits
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    let data = bits.into_bytes();
    // No MFH record -> unresolvable.
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
    // CLK, cur_mfh_id == 1, frame_size_override_flag == 1 (explicit dims), but NO
    // resolved MFH record: tile_info() / quantization_params() parse from the
    // explicit size, but segmentation_params() needs mfh_seg_info_present_flag
    // (§ 5.18.7.1), which is undecidable without the record — so the parser stops
    // there rather than guessing the sequence/zero arm.
    let mut bits = Bits::default();
    bits.uvlc(1); // cur_mfh_id == 1 -> no seq_header_id_in_frame_header
    bits.bit(0); // immediate_output_frame
    bits.bit(0); // implicit_output_frame
    bits.bit(1); // frame_size_override_flag == 1 (explicit dims)
    bits.f(7, 4); // order_hint
    // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
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
    // uvlc(1)=3 prefix bits, then 33 core bits, then 3 tile_info bits and the
    // 8-bit base_q_idx; segmentation_params() is not reached.
    assert_eq!(consumed, 3 + 33 + 3 + 8);
}

#[test]
fn frame_header_core_mfh_default_dims_parse_through_tile_info() {
    // CLK, cur_mfh_id == 1, frame_size_override_flag == 0, resolved MFH carrying
    // explicit 1920x1080 dims: the § 5.18.4.1 default path uses the MFH dims (no
    // frame-size bits), and tile_info()/quantization_params()/segmentation_params()
    // parse through to the deblocking stop.
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
    // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
    // frame_size(): MFH default path, no bits
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    bits.bit(1); // uniform_tile_spacing_flag (single tile)
    bits.bit(0); // increment_tile_cols_log2 = 0
    bits.bit(0); // increment_tile_rows_log2 = 0
    bits.f(70, 8); // base_q_idx
    // segmentation_params(): mfh_seg_info_present_flag == 0, seq has no info ->
    // sequence/zero arm. segmentation_enabled == 0 (no further bits).
    bits.bit(0); // segmentation_enabled
    bits.bit(0); // using_qmatrix (setup_qm_params)
    bits.bit(0); // delta_q_present (base_q_idx 70 > 0; 0 -> no further delta_q bits)
    // lossless tail: base_q_idx 70 non-lossless, no QM -> no qm_index bits; base_seq
    // has choose_tcq_per_frame / enable_parity_hiding off -> no allow_* bits.
    // deblocking_filter_params(): the resolved MFH did not signal an update
    // (mfh_deblocking_filter_update == 0), so apply[0]/[1] are read from the
    // bitstream. GDF/CDEF disabled in the minimal-intra seq view -> no bits.
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
    // § 5.18.2 tail: tx_mode_select f(1) + reduced_tx_set f(2); grain absent.
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
    // cur_mfh_id == 1, resolved MFH with NO frame-size payload: § 5.18.2 (:4101)
    // infers the default dims to the sequence maxima (base_seq: 4096x2304).
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
    // base_q_idx == 0 -> delta_q_present inferred 0 (no bit). Lossless tail: every
    // segment lossless, so no qm_index bits; then allow_tcq / allow_parity_hiding
    // gated off in base_seq -> no bits.
    // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
    // § 5.18.2 tail: CodedLossless == 1 -> read_tx_mode() reads NO bit (TxMode =
    // ONLY_4X4); reduced_tx_set f(2) is still read; grain absent.
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
    // CodedLossless == 1 here, so deblocking_filter_params() returns with all
    // apply flags 0 and GDF/CDEF stay disabled, all without reading bits.
    assert!(core.lossless_info.as_ref().unwrap().coded_lossless);
    assert_eq!(
        core.deblocking_filter_params
            .unwrap()
            .apply_deblocking_filter,
        [false; 4]
    );
    assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
    // The CodedLossless gate skipped tx_mode_select: TxMode is ONLY_4X4.
    let tail = core.intra_tail.as_ref().unwrap();
    assert_eq!(tail.tx_mode, TxMode::Only4x4);
    assert_eq!(tail.reduced_tx_set, 0);
}

#[test]
fn frame_header_core_mfh_segmentation_arm_reuses_mfh_feature_data() {
    // cur_mfh_id == 1, frame_size_override_flag == 1, resolved MFH with
    // mfh_seg_info_present_flag == 1, mfh_ext_seg_flag == enable_ext_seg (false),
    // mfh_allow_seg_info_change == 0: § 5.18.7.1 selects the MFH arm with
    // haveSegParams == 1, allowChange == 0, so reuse_seg_info is inferred 1 (no bit)
    // and FeatureData copies MfhFeatureData[cur_mfh_id].
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
    // segmentation_params(): MFH arm, haveSegParams==1, allowChange==0 ->
    // reuse_seg_info inferred 1, no reuse bit, copy MFH features.
    bits.bit(1); // segmentation_enabled
    // setup_qm_params(): using_qmatrix off.
    bits.bit(0); // using_qmatrix
    // delta_q_params(): base_q_idx 70 > 0.
    bits.bit(0); // delta_q_present
    // lossless tail: segment 3 has alt-q feature data 7 -> non-lossless; others
    // disabled (qindex == base_q_idx 70, non-lossless). No QM -> no qm_index bits.
    // deblocking_filter_params(): not lossless, MFH did not signal an update ->
    // apply[0]/[1] read. GDF/CDEF disabled in the minimal-intra seq view.
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
    // § 5.18.2 tail: tx_mode_select f(1) + reduced_tx_set f(2); grain absent.
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
    // The full § 5.18.2 intra tail in spec order: a 2x1-tile tile_info() with
    // context fields, quantization_params(), segmentation_params() (enabled,
    // fresh all-disabled seg_info), setup_qm_params() with two QM sets,
    // delta_q_params(), per-segment qm_index reads, allow_tcq,
    // allow_parity_hiding, and the loop-filter cluster
    // deblocking_filter_params() / gdf_params() / cdef_params() with both GDF and
    // CDEF enabled.
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
    // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
    bits.f(1920 - 1, 12); // frame_width_minus_1
    bits.f(1080 - 1, 12); // frame_height_minus_1
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    // tile_info() (§ 5.18.7.2): 1920x1080 with 128x128 superblocks (sbCols = 15,
    // sbRows = 9), one column increment -> TileCols = 2 (starts 0, 8), 1 row.
    bits.bit(1); // uniform_tile_spacing_flag
    bits.bit(1); // increment_tile_cols_log2 = 1
    bits.bit(0); // increment_tile_cols_log2 = 0
    bits.bit(0); // increment_tile_rows_log2 = 0
    bits.f(1, 1); // context_update_tile_id f(TileRowsLog2 + TileColsLog2 == 1)
    bits.f(3, 2); // tile_size_bytes_minus_1 -> TileSizeBytes = 4
    // quantization_params() (§ 5.18.6.1).
    bits.f(40, 8); // base_q_idx
    // segmentation_params() (§ 5.18.7.1): enabled, no sequence info ->
    // reuse_seg_info inferred 0, fresh seg_info(8) with all features disabled.
    bits.bit(1); // segmentation_enabled
    for _ in 0..8 {
        bits.f(0, 3); // seg_info: feature_enabled[i][0..3] = 0
    }
    // setup_qm_params() (§ 5.18.6.2): segmentation_enabled gates pic_qm_num.
    bits.bit(1); // using_qmatrix
    bits.f(1, 2); // pic_qm_num_minus_1 -> qmNum = 2
    bits.f(3, 4); // qm_y[0]
    bits.bit(1); // qm_uv_same_as_y[0]
    bits.f(5, 4); // qm_y[1]
    bits.bit(1); // qm_uv_same_as_y[1]
    // delta_q_params() (§ 5.18.7.8).
    bits.bit(0); // delta_q_present
    // § 5.18.2 lossless tail: every segment has qindex 40 (non-lossless), so each
    // of the 8 segments reads qm_index f(CeilLog2(2) == 1) == 1.
    for _ in 0..8 {
        bits.bit(1); // qm_index
    }
    bits.bit(0); // allow_tcq (choose_tcq_per_frame)
    bits.bit(1); // allow_parity_hiding
    // deblocking_filter_params() (§ 5.18.5.2): not lossless, df_par_bits_minus_2 == 0
    // -> dfParBits = 2. apply[0]=1, apply[1]=0, NumPlanes 3 + luma set -> apply[2]/[3]
    // read.
    bits.bit(1); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    bits.bit(0); // apply_deblocking_filter[2]
    bits.bit(0); // apply_deblocking_filter[3]
    // i == 0 applies: df_delta_q_present[0]=1, df_delta_q[0] f(2)==3 -> 3-2==1.
    bits.bit(1); // df_delta_q_present[0]
    bits.f(3, 2); // df_delta_q[0]
    // i == 1: apply==0 -> DfDeltaQ[1] = DfDeltaQ[0] == 1 (no bits).
    // i == 2/3: apply==0 -> DfDeltaQ == 0 (no bits).
    // gdf_params() (§ 5.18.7.9): not single picture -> gdf_frame_enable f(1)==1.
    // SbSize 128x128, MiCols(480)*4 == 1920 > gdfBlkSize(128) -> gdf_per_block f(1).
    bits.bit(1); // gdf_frame_enable
    bits.bit(0); // gdf_per_block
    bits.f(2, 2); // gdf_pic_qc_idx
    bits.f(3, 2); // gdf_pic_scale_idx -> GdfPixScale = 4
    // cdef_params() (§ 5.18.7.10): not single picture -> cdef_frame_enable f(1)==1.
    bits.bit(1); // cdef_frame_enable
    bits.f(1, 2); // cdef_damping_minus_3 -> CdefDamping = 4
    bits.f(0, 3); // cdef_strengths_minus_1 -> CdefStrengths = 1
    bits.bit(1); // cdef_on_skip_txfm_frame_enable (adaptive -> read)
    bits.bit(0); // cdef_y_pri_zero -> read f(4)
    bits.f(9, 4); // cdef_y_pri_strength[0]
    bits.f(1, 2); // cdef_y_sec_strength[0]
    bits.bit(1); // cdef_uv_pri_zero -> 0
    bits.f(3, 2); // cdef_uv_sec_strength[0] == 3 -> 4
    // lr_params()/ccso_params(): restoration and CCSO disabled (base_seq) -> no bits.
    // § 5.18.2 tail: read_tx_mode() not lossless -> tx_mode_select f(1); reduced_tx_set
    // f(2); film_grain_config() grain absent -> apply_grain inferred 0, no bits.
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
    // Every segment selected QM set 1 (qm_uv_same_as_y -> [5, 5, 5]).
    assert!(lossless.seg_qm_levels[..8].iter().all(|l| *l == [5, 5, 5]));
    assert!(!lossless.allow_tcq);
    assert!(lossless.allow_parity_hiding);
    // deblocking_filter_params(): apply[0] set, df_delta_q[0] == 1; apply[1..4] == 0
    // so DfDeltaQ[1..4] take the outer-else 0.
    let deblocking = core.deblocking_filter_params.unwrap();
    assert_eq!(
        deblocking.apply_deblocking_filter,
        [true, false, false, false]
    );
    assert_eq!(deblocking.df_delta_q_present, [true, false, false, false]);
    assert_eq!(deblocking.df_delta_q, [1, 0, 0, 0]);
    // gdf_params(): frame-enabled, per-block 0, qc 2, scale 3.
    let gdf = core.gdf_params.unwrap();
    assert!(gdf.gdf_frame_enable);
    assert_eq!(gdf.gdf_per_block, Some(false));
    assert_eq!(gdf.gdf_pic_qc_idx, Some(2));
    assert_eq!(gdf.gdf_pic_scale_idx, Some(3));
    // cdef_params(): one strength set, CdefDamping 4, y_sec remap 1, uv_sec 3->4.
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
    // lr_params(): restoration disabled (base_seq) -> Parsed with uses_lr == false and
    // no per-plane reads. ccso_params(): CCSO disabled -> ccso_frame_flag None, no reads.
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
    // 2 prefix bits + 33 control/size bits + 64 pre-filter structure bits (7 tile_info,
    // 8 base_q_idx, 25 segmentation, 13 setup_qm, 1 delta_q_present, 8 qm_index,
    // 1 allow_tcq, 1 allow_parity_hiding) + 30 loop-filter bits (7 deblocking,
    // 6 gdf, 17 cdef) + 0 lr/ccso bits (both disabled) + 3 tail bits (tx_mode_select +
    // reduced_tx_set; grain absent).
    assert_eq!(consumed, 2 + 33 + 64 + 30 + 3);
}

#[test]
fn frame_header_core_eof_inside_intra_structures() {
    // The payload ends right after disable_cdf_update: the § 5.18.2 structure
    // cluster needs at least 14 more bits, so the parse reports a typed EOF.
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
    // Regular tile group, frame_is_inter == 0 -> INTRA_ONLY_FRAME; refresh_frame_flags
    // is read as f(NumRefFrames) (no short-refresh mode).
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
    // Intra structure cluster (4096x2304, 128x128 superblocks: sbCols = 32,
    // sbRows = 18, single uniform tile).
    bits.bit(1); // uniform_tile_spacing_flag
    bits.bit(0); // increment_tile_cols_log2 = 0
    bits.bit(0); // increment_tile_rows_log2 = 0
    bits.f(45, 8); // base_q_idx
    bits.bit(0); // segmentation_enabled
    bits.bit(0); // using_qmatrix
    bits.bit(0); // delta_q_present
    // deblocking_filter_params(): not lossless -> apply[0]/[1] read (GDF/CDEF off).
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
    // § 5.18.2 tail: tx_mode_select f(1) + reduced_tx_set f(2); grain absent.
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
    // single_picture_header_flag skips the frame-type/output block; frame_size uses
    // the default (max) dimensions.
    let mut seq = base_seq();
    seq.single_picture_header_flag = true;
    seq.filter.single_picture_header_flag = true;
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    // single_picture: no type/output bits; frame_size_override_flag inferred 0
    bits.f(9, 4); // order_hint
    // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    // Intra structure cluster (4096x2304 single uniform tile, see above).
    bits.bit(1); // uniform_tile_spacing_flag
    bits.bit(0); // increment_tile_cols_log2 = 0
    bits.bit(0); // increment_tile_rows_log2 = 0
    bits.f(45, 8); // base_q_idx
    bits.bit(0); // segmentation_enabled
    bits.bit(0); // using_qmatrix
    bits.bit(0); // delta_q_present
    // deblocking_filter_params(): not lossless -> apply[0]/[1] read. GDF/CDEF are
    // disabled in the minimal-intra seq view, so the single-picture enable inference is not reached.
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
    // §5.18.2 tail: read_tx_mode() not lossless -> tx_mode_select f(1); reduced_tx_set
    // f(2); film_grain_config() grain absent (film_grain_params_present == false) ->
    // apply_grain inferred 0 even though single_picture_header_flag is set, since the
    // first gate (!film_grain_params_present) wins.
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
    // AV2 § 5.18.2: an OBU_BRIDGE_FRAME whose sequence has single_picture_header_flag == 1
    // reads bridge_frame_ref_idx FIRST (the `if ( IsBridge )` block at mirror :4117, BEFORE
    // the single-picture branch at :4131), then the single-picture branch forces FrameType =
    // KEY_FRAME / FrameIsIntra = 1 / immediate_output_frame = 1 (:4135-4139). It is a HYBRID,
    // NOT the full intra key path: because IsBridge == 1 it reads bridge_frame_overwrite_flag
    // f(1) (:4423), then the OVERWRITE-GATED refresh_frame_flags (§ 6.17.2 + AVM: overwrite == 0
    // -> inferred 1 << bridge_frame_ref_idx, NO bits), then — FrameIsIntra == 1 — frame_size()
    // (override 0 -> default dims, no bits, :4567), screen_content_params() (:4569) and
    // intrabc_params() (:4571), and the decidable film_grain_config() tail (here grain absent ->
    // 0 bits). It then reaches the `if ( ... || IsBridge )` early-return arm (:4971) where
    // base_q_idx = RefBaseQIdx[bridge_frame_ref_idx] is reference-derived (:4997) and
    // disable_cdf_update (the :5039 else-arm) + the whole quant/segmentation/deblocking/cdef/
    // ccso cluster (:5045-5083) are SKIPPED. So the parse stops honestly with
    // InterStop::BruInactiveOrBridgeReturn — NOT IntraHeaderComplete.
    //
    // This replaces the pre-fix test whose premise was the bug: the parser used to route a
    // single-picture bridge through the FULL intra path (parse_intra_tail), reading order_hint,
    // disable_cdf_update and the entire structure cluster and reaching a bogus
    // IntraHeaderComplete, and never reading bridge_frame_overwrite_flag.
    //
    // Refresh reading (documented in openspec/changes/frame-header-single-picture-bridge-fix):
    // § 5.18.2 syntax and § 6.17.2 semantics CONTRADICT for this corner; splot follows
    // § 6.17.2 + AVM (overwrite-gated), so for overwrite == 0 refresh_frame_flags is INFERRED
    // 1 << bridge_frame_ref_idx with no bits.
    let mut seq = base_seq();
    seq.single_picture_header_flag = true;
    seq.filter.single_picture_header_flag = true;
    let mut bits = Bits::default();
    // Bridge prefix: cur_mfh_id inferred 0 (no bits), seq_header_id_in_frame_header.
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(5, 3); // bridge_frame_ref_idx = 5 f(CeilLog2(8) == 3) — read before single-pic
    // IsBridge prefix (mirror :4423-4571), reached on the FrameIsIntra arm:
    bits.bit(0); // bridge_frame_overwrite_flag = 0 f(1) (mirror :4423)
    // refresh_frame_flags: overwrite == 0 -> inferred 1 << 5 = 32, NO bits (§ 6.17.2 + AVM).
    // frame_size(): override 0, cur_mfh_id == 0 -> default max dims (4096x2304), no bits.
    // screen_content_params(): seq_force off -> no bits.
    bits.bit(0); // allow_intrabc = 0 f(1) (intrabc_params(), mirror :4571)
    // film_grain_config(): film_grain_params_present == false (base_seq) -> apply_grain 0, no bits.
    // STOP: IsBridge early-return arm (mirror :4971). No disable_cdf_update, no cluster.
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::BridgeFrame, true, &seq).unwrap();

    assert!(core.is_bridge, "the OBU is still an OBU_BRIDGE_FRAME");
    assert_eq!(
        core.bridge_frame_ref_idx,
        Some(5),
        "bridge_frame_ref_idx is read before the single-picture branch (mirror :4117)"
    );
    // The single-picture branch forces the KEY/intra/output state.
    assert_eq!(core.show_existing_frame, Some(false));
    assert_eq!(core.frame_type, Some(FrameType::Key));
    assert_eq!(core.frame_is_intra, Some(true));
    assert_eq!(
        core.immediate_output_frame,
        Some(true),
        "single_picture forces immediate_output_frame = 1"
    );
    assert_eq!(core.implicit_output_frame, Some(false));
    // The IsBridge prefix reads (mirror :4423-4575) are recorded on core.inter.
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
    assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
    assert_eq!(core.allow_screen_content_tools, Some(false));
    assert_eq!(core.allow_intrabc, Some(false));
    // The IsBridge early-return arm SKIPS disable_cdf_update and the whole structure cluster.
    assert_eq!(
        core.disable_cdf_update, None,
        "the IsBridge early-return arm never reads disable_cdf_update (mirror :4971/:5039)"
    );
    assert!(
        core.tile_info.is_none(),
        "no quant/tile structure cluster on the IsBridge early-return arm"
    );
    assert!(core.quantization_params.is_none());
    assert!(
        core.intra_tail.is_none(),
        "the full intra tail is NOT taken for a single-picture bridge"
    );
    assert_eq!(
        inter.stop,
        Some(InterStop::BruInactiveOrBridgeReturn),
        "stops at the § 5.18.2 IsBridge early-return arm (mirror :4971), not IntraHeaderComplete"
    );
    assert!(matches!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
    ));
}

#[test]
fn frame_header_core_single_picture_bridge_reads_scc_and_intrabc_conditionals() {
    // overwrite == 1: refresh_frame_flags IS read (§ 6.17.2 + AVM gate it on overwrite). With
    // enable_short_refresh_frame_flags this is the AVM bridge short path — has_refresh_frame_flags
    // f(1) + frame_to_refresh f(CeilLog2(NumRefFrames)). This also exercises the data-dependent
    // screen_content / intrabc reads on the FrameIsIntra arm (mirror :4569-4571).
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
    // frame_size(): override 0 -> default dims, no bits.
    bits.bit(1); // allow_screen_content_tools = 1 (mirror :4569 / §5.18.3.3)
    bits.bit(1); // force_integer_mv = 1 (allow_sct && seq_force_integer_mv == SELECT)
    bits.bit(1); // allow_intrabc = 1 (mirror :4571 / §5.18.3.4)
    bits.bit(1); // allow_global_intrabc = 1 (allow_intrabc && FrameIsIntra)
    bits.bit(0); // allow_local_intrabc = 0 (allow_global_intrabc == 1 -> read)
    // allow_frame_max_bvp_drl_bits == false -> no change_bvp_drl. STOP at bridge return.
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
    // A payload that ends inside the modeled single-picture-bridge prefix is a decidable
    // truncation, not a hard parse error: finish_inter_control preserves the fields parsed
    // before the EOF and reports StoppedInsideInterControl (codex F2). With overwrite == 1 the
    // refresh_frame_flags read IS reached (the long arm, enable_short off -> f(NumRefFrames)),
    // and the payload ends inside it.
    let mut seq = base_seq();
    seq.single_picture_header_flag = true;
    seq.filter.single_picture_header_flag = true;
    let mut bits = Bits::default();
    bits.uvlc(0); // seq_header_id_in_frame_header (1 bit)
    bits.f(5, 3); // bridge_frame_ref_idx = 5 (3 bits)
    bits.bit(1); // bridge_frame_overwrite_flag = 1 (1 bit) -> 5 bits, padded to 1 byte
    // refresh_frame_flags f(NumRefFrames == 8) starts at bit 5 with only 3 padding bits -> EOF.
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
    // When film_grain_params_present is set, the IsBridge early-return arm's film_grain_config()
    // (mirror :5011 / §5.18.10.1) infers apply_grain = 1 (single_picture + immediate_output == 1,
    // mirror :8169-8171) and reads fgm_id f(3) + grain_seed f(16) with NO reference state — the
    // LAST modeled frame-header bits. The parser consumes that mandatory tail (so consumed_bits
    // is complete) before the BruInactiveOrBridgeReturn stop. (A non-single bridge has
    // immediate_output == 0 -> apply_grain == 0 -> no grain bits, which is why only the
    // single-picture bridge reads them.)
    let mut seq = base_seq();
    seq.single_picture_header_flag = true;
    seq.filter.single_picture_header_flag = true;
    seq.film_grain_params_present = Some(true);
    let mut bits = Bits::default();
    bits.uvlc(0); // seq_header_id_in_frame_header (1 bit)
    bits.f(5, 3); // bridge_frame_ref_idx = 5 (3 bits)
    bits.bit(0); // bridge_frame_overwrite_flag = 0 (1 bit) -> refresh inferred 1 << 5, no bits
    // frame_size(): 0 bits. screen_content_params(): seq_force off -> 0 bits.
    bits.bit(0); // allow_intrabc = 0 (1 bit)
    // IsBridge early-return arm: tile_info() 0 bits; base_q_idx inferred (no bits);
    // film_grain_config(): apply_grain inferred 1 -> fgm_id f(3) + grain_seed f(16).
    bits.f(5, 3); // fgm_id = 5
    bits.f(0xBEEF, 16); // grain_seed
    let data = bits.into_bytes();
    let (core, consumed) = parse_body(&data, ObuType::BridgeFrame, true, &seq).unwrap();

    assert!(core.is_bridge);
    let inter = core.inter.as_ref().expect("bridge facts recorded");
    assert_eq!(
        inter.refresh_frame_flags,
        Some(1 << 5),
        "overwrite == 0 -> refresh inferred (no bits)"
    );
    assert_eq!(inter.stop, Some(InterStop::BruInactiveOrBridgeReturn));
    assert!(matches!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
    ));
    // 1 (seq id) + 3 (bridge ref) + 1 (overwrite) + 0 (refresh inferred) + 1 (allow_intrabc)
    // + 3 (fgm_id) + 16 (grain_seed) = 25 bits: the mandatory grain tail is accounted for.
    assert_eq!(consumed, 25, "consumed_bits covers the film-grain tail");
}

#[test]
fn frame_header_core_single_picture_bridge_eof_in_film_grain_is_truncation() {
    // A truncation inside the mandatory film-grain tail of a grain-enabled single-picture bridge
    // is a decidable defect (no reference state is needed to know those bits must be present), so
    // it is reported as StoppedInsideInterControl, not a silent coverage stop (codex review).
    let mut seq = base_seq();
    seq.single_picture_header_flag = true;
    seq.filter.single_picture_header_flag = true;
    seq.film_grain_params_present = Some(true);
    let mut bits = Bits::default();
    bits.uvlc(0); // seq_header_id (1 bit)
    bits.f(5, 3); // bridge_frame_ref_idx (3 bits) -> 4
    bits.bit(0); // bridge_frame_overwrite_flag = 0 (1 bit) -> 5; refresh inferred (no bits)
    bits.bit(0); // allow_intrabc (1 bit) -> 6
    bits.f(5, 3); // fgm_id f(3) -> 9; grain_seed f(16) then runs out of bits -> EOF.
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::BridgeFrame, true, &seq).unwrap();

    assert_eq!(
        core.status,
        FrameHeaderParseStatus::StoppedInsideInterControl
    );
    let inter = core.inter.as_ref().expect("pre-EOF facts preserved");
    assert_eq!(inter.bridge_frame_overwrite_flag, Some(false));
    assert_eq!(
        inter.refresh_frame_flags,
        Some(1 << 5),
        "the inferred refresh (overwrite == 0) parsed before the grain-tail EOF is preserved"
    );
    assert_eq!(
        core.frame_size,
        Some(FrameSize::new(4096, 2304)),
        "facts parsed before the grain-tail EOF are preserved"
    );
    assert_eq!(
        inter.stop, None,
        "the bridge-return stop was never reached (EOF inside the grain tail)"
    );
}

#[test]
fn frame_header_core_bridge_parses_overwrite_refresh_and_size_arms() {
    // Bridge frame: cur_mfh_id inferred 0, reads seq_header_id, bridge_frame_ref_idx
    // f(CeilLog2(8) == 3), then the IsBridge reference-control arms (AV2 § 5.18.2,
    // mirror :4425-4633): bridge_frame_overwrite_flag f(1) == 0 -> refresh = 1 <<
    // bridge_frame_ref_idx (no bits), NumTotalRefs = 1 / ref_frame_idx = bridge (no
    // bits), then frame_size_with_bridge() Min(ref dims, explicit dims). The IsBridge
    // early-return arm (mirror :4971/:5045) then stops.
    let mut bits = Bits::default();
    bits.uvlc(4); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
    bits.f(5, 3); // bridge_frame_ref_idx = 5
    bits.bit(0); // bridge_frame_overwrite_flag = 0 -> refresh = 1 << 5 (no bits)
    bits.f(1920 - 1, 12); // bridge_frame_width_minus_1
    bits.f(1080 - 1, 12); // bridge_frame_height_minus_1
    let data = bits.into_bytes();

    // RefFrameWidth/Height[5] modeled so frame_size_with_bridge() Min resolves.
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
    // frame_size_with_bridge() Min(1280, 1920) x Min(720, 1080).
    assert_eq!(core.frame_size, Some(FrameSize::new(1280, 720)));
    // The bridge takes the IsBridge early-return arm.
    assert_eq!(inter.stop, Some(InterStop::BruInactiveOrBridgeReturn));
    assert!(matches!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
    ));
}

#[test]
fn frame_header_core_bridge_overwrite_reads_refresh_frame_flags() {
    // Bridge frame with bridge_frame_overwrite_flag == 1 takes the else refresh arm
    // (AV2 § 5.18.2 mirror :4533): refresh_frame_flags f(NumRefFrames == 8).
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
fn frame_header_core_show_existing_frame_reads_map_idx_and_order_hint() {
    // Regular SEF: ShowExistingFrame == 1; reads frame_to_show_map_idx f(3),
    // derive_sef_order_hint f(1) == 0, then sef_order_hint f(OrderHintBits == 4).
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
    // base_seq() has film_grain_params_present == false, so film_grain_config()
    // infers apply_grain = 0 (no bit) and the SEF header completes.
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::ShowExistingFrameComplete
    );
    let fg = core.sef_film_grain.expect("SEF film_grain_config parsed");
    assert!(!fg.apply_grain);
    assert_eq!(fg.fgm_id, None);
    // A conformant grain-free SEF tail classifies as Valid (no diagnostic).
    assert_eq!(core.sef_trailing_bits, Some(SefTrailingBits::Valid));
}

#[test]
fn frame_header_core_show_existing_frame_derives_order_hint() {
    // derive_sef_order_hint == 1: sef_order_hint is not read; OrderHintLsbs is
    // derived from the referenced slot (reference state), so it is left unknown.
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
    // Grain not present -> apply_grain inferred 0, SEF header completes.
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::ShowExistingFrameComplete
    );
    assert!(core.sef_film_grain.is_some());
    assert_eq!(core.sef_trailing_bits, Some(SefTrailingBits::Valid));
}

#[test]
fn frame_header_core_inter_implicit_map_stops_unmodeled() {
    // Regular tile group, frame_is_inter == 1 -> INTER_FRAME. With the sequence's
    // explicit_ref_frame_map off, explicitRefFrameMap derives 0 and get_ref_frames(0)
    // is unmodeled, so the inter region stops honestly right after the refresh flags.
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
    // explicit_ref_frame_map seq flag off -> explicitRefFrameMap 0 -> get_ref_frames(0).
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
    // Regular tile group, INTER, with the sequence explicit_ref_frame_map on: the
    // inter control region parses the explicit map, frame size, MV precision, the
    // interpolation filter, and motion modes, converging into the shared tail
    // (InterStop::ReachedSharedTail). The hand-built payload ends EXACTLY at that
    // boundary (right after disable_cdf_update), so the parse now CONTINUES into the
    // § 5.18.2 shared tail (tile_info() onward) and runs out of bits inside it — a
    // facts-preserving truncation (StoppedInsideInterControl), with the control-region
    // facts intact. (The positive end-to-end completion is proven against the real
    // fixture in inter.rs::frame_header_core_inter_fixture_reaches_inter_header_complete.)
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
    // non-override, cur_mfh_id == 0 -> frame_size() default dims (no bits).
    bits.bit(0); // use_ref_frame_mvs (num_total_refs == 1 -> no tmvp)
    bits.bit(0); // allow_intrabc
    bits.bit(0); // use_qtr_precision_mv
    bits.bit(0); // allow_high_precision_mv
    bits.bit(1); // is_filter_switchable
    // motion modes: seq_frame_motion_modes_present_flag false -> no bits.
    bits.bit(0); // disable_cdf_update f(1) (mirror :5041), just before the shared tail.
    let data = bits.into_bytes();
    let (core, _) = parse_body(&data, ObuType::RegularTileGroup, true, &seq).unwrap();

    assert_eq!(core.frame_type, Some(FrameType::Inter));
    let inter = core.inter.as_ref().unwrap();
    // The control region's facts are all preserved through the shared-tail truncation.
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
    // The payload ends at the shared-tail boundary, so continuing into tile_info() runs
    // out of bits: a facts-preserving truncation in the modeled region.
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
    // Implicit reference map (the fixture path) + ref-frame-mvs available.
    seq.inter.explicit_ref_frame_map = false;
    seq.inter.enable_ref_frame_mvs = true;
    seq.filter.enable_df_sub_pu = true;
    // A reusable uniform single tile so tile_info() reads 0 bits (the fixture layout).
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
    // explicit_ref_frame_map seq flag off -> get_ref_frames(0) -> NumTotalRefs == 1,
    // ref_frame_idx == [0] (no bits). num_total_refs == 1 -> no tmvp.
    bits.bit(0); // use_ref_frame_mvs = 0
    // non-override, cur_mfh_id == 0 -> frame_size() default dims (no bits).
    // TIP gate false (enable_tip off). frame_opfl_refine_type(): no bits.
    bits.bit(0); // intrabc_params(): allow_intrabc = 0
    bits.bit(0); // use_qtr_precision_mv = 0
    bits.bit(0); // allow_high_precision_mv = 0 -> HALF_PEL
    bits.bit(1); // is_filter_switchable = 1
    // motion modes: seq_frame_motion_modes_present_flag false -> no bits.
    bits.bit(0); // disable_cdf_update f(1), the shared-tail boundary.
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
fn frame_header_core_inter_shared_tail_reads_inter_arms_with_asymmetric_values() {
    // A COMPLETE minimal-tool inter header: the control prefix + the § 5.18.2 shared tail
    // with ASYMMETRIC inter-tail values (reference_select == 1, skip_mode_present == 1,
    // tx_mode_select == 1) so a swap of the two adjacent f(1) reads would be caught (the
    // asymmetric-value discipline). Reaches InterHeaderComplete with the exact values.
    let seq = minimal_inter_seq_64();
    let mut bits = Bits::default();
    minimal_inter_control_prefix(&mut bits);
    // --- shared tail ---
    // tile_info(): reusable uniform 1x1 -> 0 bits.
    bits.f(90, 8); // quantization_params(): base_q_idx f(8) (asymmetric, != 0)
    bits.bit(0); // segmentation_params(): segmentation_enabled = 0
    bits.bit(0); // setup_qm_params(): using_qmatrix = 0
    bits.bit(0); // delta_q_params(): base_q>0 -> delta_q_present = 0
    // lossless: enable_tcq/parity off -> no bits.
    // deblocking_filter_params(): inter, enable_df_sub_pu on -> allow_df_sub_pu f(1).
    bits.bit(0); // allow_df_sub_pu
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    // both 0 -> no chroma pair, no df_delta_q. gdf/cdef/lr/ccso disabled -> 0 bits.
    bits.bit(1); // read_tx_mode(): tx_mode_select = 1 -> TX_MODE_SELECT
    bits.bit(1); // frame_reference_mode(): reference_select = 1
    bits.bit(1); // skip_mode_params(): skip_mode_present = 1 (skipModeAllowed)
    // allow_bawp: enable_bawp off -> no bit. allow_warpmv_mode: no DELTAWARP -> no bit.
    bits.f(2, 2); // reduced_tx_set f(2) = 2
    // global_motion_params(): enable_global_motion off -> intra-arm return, no bits.
    bits.bit(0); // film_grain_config(): apply_grain = 0 (output frame, grain present)
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
    assert!(!tail.use_global_motion);
    assert!(!tail.apply_grain);
}

#[test]
fn frame_header_core_inter_shared_tail_segmentation_on_stops_unmodeled() {
    // segmentation_enabled == 1 on the inter path: the § 5.18.7.1 update_map /
    // temporal arms depend on the unmodeled DerivedPrimaryRefFrame ranking, so the
    // shared tail stops honestly at UnsupportedUntilFeature (a coverage stop, not a
    // truncation), with the control-region facts preserved.
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
    // The control region reached the shared tail; the enabled-segmentation stop happens
    // before the shared facts are stored, so no inter tail is produced.
    assert_eq!(
        core.inter.as_ref().unwrap().stop,
        Some(crate::headers::frame::inter::InterStop::ReachedSharedTail)
    );
    assert!(core.inter_tail.is_none());
}

#[test]
fn frame_header_core_inter_shared_tail_ccso_on_stops_before_any_tail_bit() {
    // enable_ccso == true puts the inter ccso reuse arm in play, which the shared
    // (intra-arm) parser does not model. The admission gate stops BEFORE reading any
    // shared-tail bit (setup_qm stays None), so no possibly-misaligned using_qmatrix is
    // ever exposed. The control region's facts are still preserved.
    let mut seq = minimal_inter_seq_64();
    seq.ccso.enable_ccso = true;
    let mut bits = Bits::default();
    minimal_inter_control_prefix(&mut bits);
    // The shared tail would follow, but the admission gate stops first; pad anyway.
    bits.f(0, 16);
    let data = bits.into_bytes();
    let (rv, roh, rw, rh) = one_valid_ref_64();
    let rs = FrameReferenceStateView::from_slots(&rv, &roh, &rw, &rh);
    let (core, _) =
        parse_body_with_ref(&data, ObuType::RegularTileGroup, false, &seq, None, &rs).unwrap();
    assert!(matches!(
        core.status,
        FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
    ));
    assert!(
        core.quantization_params.is_none() && core.setup_qm_params.is_none(),
        "the admission gate stops before any shared-tail bit, so no setup_qm is exposed"
    );
    // The control region reached the shared tail (the precondition for the gate).
    assert_eq!(
        core.inter.as_ref().unwrap().stop,
        Some(crate::headers::frame::inter::InterStop::ReachedSharedTail)
    );
}

#[test]
fn frame_header_core_inter_eof_inside_control_region_is_truncation() {
    // Codex F2: an inter frame whose payload ends INSIDE the modeled § 5.18.2 control
    // region (here right after num_total_refs, before ref_frame_idx[0]) must surface as a
    // facts-preserving truncation (StoppedInsideInterControl), NOT propagate
    // UnexpectedEof out of parse_frame_header_core. The region is fully modeled up to its
    // coverage stops, so the EOF is a decidable bitstream defect — the validator routes
    // it to frame-header/truncated-frame-header. Pre-fix the `?` propagated the error and
    // the validator's `.ok()` dropped every fact and the truncation (the PR #57/#59 class).
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
    // The stream ends here (24 bits == 3 bytes); ref_frame_idx[0] f(3) hits EOF.
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
    // The facts parsed before the EOF survive (the regression: they were dropped pre-fix).
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
    // Codex F2 (bridge arm): an OBU_BRIDGE_FRAME whose payload ends inside the modeled
    // bridge control region (here inside frame_size_with_bridge() after
    // bridge_frame_overwrite_flag) must surface as StoppedInsideInterControl with the
    // already-parsed bridge facts preserved, not propagate UnexpectedEof.
    let mut bits = Bits::default();
    bits.uvlc(0); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
    bits.f(5, 3); // bridge_frame_ref_idx = 5 f(CeilLog2(8) == 3)
    bits.bit(0); // bridge_frame_overwrite_flag = 0 -> refresh = 1 << 5 (no bits)
    // frame_size_with_bridge() reads bridge_frame_width_minus_1 f(12); truncate inside it.
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
fn frame_header_core_ras_reads_num_key_ref_frames_then_stops() {
    // RAS frame: restricted_prediction_switch f(1), then (long_term_frame_id_bits
    // != 0) num_key_ref_frames f(3) and the ref_long_term_id loop, then the inter
    // output-control flags and order_hint, before the RAS refresh derivation
    // (max_mlayer_id == 0) stops honestly (it reads RefValid / RefLongTermId).
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
    // frame_size_override_flag forced 1 for SWITCH (no bit).
    bits.f(3, 4); // order_hint f(OrderHintBits == 4)
    // RAS + max_mlayer_id == 0 -> refresh_frame_flags derivation reads RefValid (no
    // bits), stop honestly.
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
    // ref_long_term_id values 5 and 9 are not the reserved (1 << 4) - 1 == 15.
    assert!(!core.forbidden_ref_long_term_id);
}

#[test]
fn frame_header_core_flags_reserved_ref_long_term_id() {
    // A ref_long_term_id equal to (1 << long_term_frame_id_bits) - 1 is reserved
    // (AV2 § 6.17.2); the parser records the violation for the validator.
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
    // num_key_ref_frames == 7 (7 * 4 = 28 bits) overruns the payload, which ends
    // right after num_key_ref_frames.
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
    // OLK: FrameType::Key reads long_term_id_plus_1 f(4), then (long_term_frame_id_bits
    // != 0) num_key_ref_frames f(3) + the ref_long_term_id loop, then continues into
    // the intra tail. Unlike CLK, OLK is not the `obu_type == OBU_CLOSED_LOOP_KEY`
    // allFrames case, so refresh_frame_flags is read as f(NumRefFrames) (AV2 § 5.18.2).
    let mut seq = base_seq();
    seq.long_term_frame_id_bits = 4;
    let mut bits = Bits::default();
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(0); // seq_header_id_in_frame_header
    bits.f(1, 4); // long_term_id_plus_1
    bits.f(1, 3); // num_key_ref_frames == 1
    bits.f(3, 4); // ref_long_term_id[0]
    // immediate_output_frame: OLK forces false (no bit)
    bits.bit(0); // implicit_output_frame
    bits.bit(0); // frame_size_override_flag (cur_mfh_id == 0 -> max dims)
    bits.f(2, 4); // order_hint
    bits.f(0b0000_0101, 8); // refresh_frame_flags f(NumRefFrames == 8)
    bits.bit(0); // allow_intrabc
    bits.bit(0); // disable_cdf_update
    // Intra structure cluster (4096x2304 single uniform tile, see above).
    bits.bit(1); // uniform_tile_spacing_flag
    bits.bit(0); // increment_tile_cols_log2 = 0
    bits.bit(0); // increment_tile_rows_log2 = 0
    bits.f(45, 8); // base_q_idx
    bits.bit(0); // segmentation_enabled
    bits.bit(0); // using_qmatrix
    bits.bit(0); // delta_q_present
    // deblocking_filter_params(): not lossless -> apply[0]/[1] read (GDF/CDEF off).
    bits.bit(0); // apply_deblocking_filter[0]
    bits.bit(0); // apply_deblocking_filter[1]
    // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
    // § 5.18.2 tail: tx_mode_select f(1) + reduced_tx_set f(2); grain absent.
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

mod tail;
