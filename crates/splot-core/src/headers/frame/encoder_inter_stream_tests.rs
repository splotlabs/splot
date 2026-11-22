// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::annexb::parse_annex_b_obus_partial;
use crate::headers::frame::GmType;
use crate::headers::frame::global_motion::DEFAULT_WARP_PARAMS;
use crate::headers::frame::{
    CcsoPlaneParams, CdefStrengthSet, CdefStrengthSets, EncoderStreamParams, EncoderToolFlags,
    FrameHeaderParseInput, FrameType, InterpolationFilter, MvPrecision, SeqTiles, TxMode,
    build_encoder_sequence_header, parse_frame_header_core,
};
use crate::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SequenceHeader, SuperblockSize};

/// The § 7.23 slot state after one closed-loop key frame refreshed every slot:
/// only slot 0 is valid (§ 7.20), with order hint `hint`.
struct Slots {
    valid: [bool; 8],
    hints: [u32; 8],
    widths: [u32; 8],
    heights: [u32; 8],
    base_q: [u32; 8],
    shown: [bool; 8],
    hidden: [bool; 8],
    counters: [u32; 8],
}

impl Slots {
    fn after_key(width: u32, height: u32, hint: u32) -> Self {
        let mut valid = [false; 8];
        valid[0] = true;
        Self {
            valid,
            hints: [hint; 8],
            widths: [width; 8],
            heights: [height; 8],
            base_q: [60; 8],
            shown: [true; 8],
            hidden: [false; 8],
            counters: [0; 8],
        }
    }

    fn view(&self) -> FrameReferenceStateView<'_> {
        FrameReferenceStateView::from_slots_with_base_q_idx(
            &self.valid,
            &self.hints,
            &self.widths,
            &self.heights,
            &self.base_q,
        )
        .with_single_layer_order_hint_state(&self.hints, &self.hidden, &self.shown)
        .with_primary_reference_state(&self.counters, &self.hidden)
    }
}

fn sequence(width: u32, height: u32, bit_depth: BitDepthIdc, sb: SuperblockSize) -> SequenceHeader {
    build_encoder_sequence_header(&EncoderStreamParams {
        width,
        height,
        bit_depth,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: sb,
        tools: EncoderToolFlags {
            cdef: true,
            loop_restoration: true,
            ccso: true,
            screen_content: sb == SuperblockSize::Block128x128,
            ..EncoderToolFlags::default()
        },
    })
    .unwrap()
}

fn filters(sb: SuperblockSize) -> FrameFilterParams {
    FrameFilterParams {
        deblocking: [true, true, true, false],
        deblocking_delta_q: [1, -2, 3, 0],
        cdef_damping: 4,
        cdef_strengths: CdefStrengthSets::from_slice(&[CdefStrengthSet {
            y_pri_strength: 4,
            y_sec_strength: 1,
            uv_pri_strength: 2,
            uv_sec_strength: 0,
        }])
        .unwrap(),
        cdef_on_skip: true,
        deblocking_sub_pu: false,
        lr_luma_unit_size: Some(match sb {
            SuperblockSize::Block64x64 => 64,
            SuperblockSize::Block128x128 => 128,
            SuperblockSize::Block256x256 => 256,
        }),
        lr_luma_wiener_ns: None,
        lr_chroma_unit_size: None,
        lr_chroma_wiener_ns: [None; 2],
        ccso: [
            CcsoPlaneParams::default(),
            CcsoPlaneParams {
                ccso_planes: true,
                ccso_bo_only: Some(true),
                ccso_scale_idx: Some(2),
                ccso_quant_idx: Some(0),
                ccso_ext_filter: Some(0),
                ccso_edge_clf: Some(false),
                ccso_max_band_log2: Some(1),
                ccso_offset_idx: crate::headers::frame::CcsoOffsets::from_iter_checked([6, 1])
                    .unwrap(),
                ..CcsoPlaneParams::default()
            },
            CcsoPlaneParams::default(),
        ],
        gdf: None,
    }
}

#[test]
fn inter_headers_parse_with_the_reference_state() {
    for (width, height) in [(1, 1), (17, 9), (200, 136), (1920, 1080)] {
        for bit_depth in [BitDepthIdc::Eight, BitDepthIdc::Ten] {
            for sb in [
                SuperblockSize::Block64x64,
                SuperblockSize::Block128x128,
                SuperblockSize::Block256x256,
            ] {
                let seq = sequence(width, height, bit_depth, sb);
                let view = CoreSeqView::from_sequence(&seq).unwrap();
                let slots = Slots::after_key(width, height, 4);
                for (show_frame, filters, compound, high_precision_mv, primary_slot) in [
                    (true, FrameFilterParams::default(), false, false, None),
                    (false, filters(sb), true, true, Some(0)),
                ] {
                    let switchable_interp_filter = compound;
                    let params = InterFrameParams {
                        base_q_idx: 90,
                        order_hint: 5,
                        refresh_frame_flags: 0b10,
                        show_frame,
                        filters,
                        tx: KeyFrameTxParams::default(),
                        tiles: FrameTiles::default(),
                        reference_select: compound,
                        skip_mode_present: compound,
                        primary_slot,
                        use_ref_frame_mvs: false,
                        high_precision_mv,
                        switchable_interp_filter,
                        tip_weight: None,
                        tip_global_mv: TipGlobalMv::default(),
                        tip_filter: None,
                        tip_deblocking: false,
                        tip_hole_fill: false,
                        opfl_refine_type: 0,
                        force_integer_mv: sb == SuperblockSize::Block128x128 && !high_precision_mv,
                        warp: false,
                        global_motion: [None; 7],
                        global_motion_predict: false,
                        explicit_refs: None,
                        implicit_output: false,
                        leading: false,
                    };
                    let header = build_inter_frame_header(&view, &slots.view(), &params).unwrap();
                    let core = &header.core;
                    assert_eq!(core.frame_type, Some(FrameType::Inter));
                    assert_eq!(core.immediate_output_frame, Some(show_frame));
                    assert_eq!(core.implicit_output_frame, Some(false));
                    assert_eq!(core.order_hint, Some(5));
                    assert_eq!(core.refresh_frame_flags, Some(0b10));
                    assert_eq!(core.frame_size.unwrap().width, width);
                    let inter = core.inter.as_ref().unwrap();
                    assert_eq!(&inter.ref_frame_idx[..], &[0]);
                    let primary = primary_slot.map_or(PRIMARY_REF_NONE as u8, |_| 0);
                    assert_eq!(inter.primary_ref_frame, Some(primary));
                    assert_eq!(
                        inter.disable_cross_frame_cdf_init,
                        Some(primary_slot.is_none())
                    );
                    let precision = if params.force_integer_mv {
                        MvPrecision::OnePel
                    } else if high_precision_mv {
                        MvPrecision::EighthPel
                    } else {
                        MvPrecision::QuarterPel
                    };
                    assert_eq!(inter.mv_precision, Some(precision));
                    let screen = sb == SuperblockSize::Block128x128;
                    assert_eq!(inter.allow_screen_content_tools, Some(screen));
                    assert_eq!(inter.force_integer_mv, Some(params.force_integer_mv));
                    let filter = if switchable_interp_filter {
                        InterpolationFilter::Switchable
                    } else {
                        InterpolationFilter::Eighttap
                    };
                    assert_eq!(inter.interpolation_filter, Some(filter));
                    let tail = core.inter_tail.as_ref().unwrap();
                    assert_eq!(tail.tx_mode, TxMode::Largest);
                    assert_eq!(tail.reference_select, compound);
                    assert_eq!(tail.skip_mode_present, compound);
                    assert_eq!(
                        tail.reduced_tx_set,
                        KeyFrameTxParams::default().reduced_tx_set
                    );
                    assert_eq!(core.quantization_params.unwrap().base_q_idx, 90);
                    assert_eq!(
                        core.cdef_params.as_ref().unwrap().cdef_frame_enable,
                        !show_frame
                    );
                    let ccso = core.ccso_params.as_ref().unwrap();
                    assert_eq!(ccso.ccso_frame_flag, Some(!show_frame));
                    let planes = if show_frame {
                        &[][..]
                    } else {
                        &filters.ccso[..]
                    };
                    assert_eq!(ccso.planes[..], *planes);
                    assert_eq!(core.consumed_bits, header.bits.num_frame_header_bits());
                }
            }
        }
    }
}

#[test]
fn order_hints_unwrap_against_the_reference_slots() {
    let seq = sequence(64, 64, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let slots = Slots::after_key(64, 64, 300);
    let params = InterFrameParams {
        base_q_idx: 90,
        order_hint: 301,
        refresh_frame_flags: 1,
        show_frame: true,
        filters: FrameFilterParams::default(),
        tx: KeyFrameTxParams::default(),
        tiles: FrameTiles::default(),
        reference_select: false,
        skip_mode_present: false,
        primary_slot: Some(3),
        use_ref_frame_mvs: false,
        high_precision_mv: false,
        switchable_interp_filter: false,
        tip_weight: None,
        tip_global_mv: TipGlobalMv::default(),
        tip_filter: None,
        tip_deblocking: false,
        tip_hole_fill: false,
        opfl_refine_type: 0,
        force_integer_mv: false,
        warp: false,
        global_motion: [None; 7],
        global_motion_predict: false,
        explicit_refs: None,
        implicit_output: false,
        leading: false,
    };
    let header = build_inter_frame_header(&view, &slots.view(), &params).unwrap();
    let inter = header.core.inter.as_ref().unwrap();
    assert_eq!(
        inter.primary_ref_frame,
        Some(PRIMARY_REF_NONE as u8),
        "slot 3 is empty"
    );
    assert_eq!(header.core.order_hint_lsb, Some(301 - 256));
    assert_eq!(header.core.order_hint, Some(301));
}

/// `use_ref_frame_mvs` parses back; over two references with 128x128
/// superblocks the header also codes `tmvp_sample_step_minus_1 = 0`. A
/// sequence without `enable_ref_frame_mvs` refuses it.
#[test]
fn reference_frame_mvs_parse_over_one_and_two_references() {
    for sb in [SuperblockSize::Block64x64, SuperblockSize::Block128x128] {
        let mut stream = EncoderStreamParams {
            width: 64,
            height: 64,
            bit_depth: BitDepthIdc::Eight,
            chroma: ChromaFormatIdc::Yuv420,
            sb_size: sb,
            tools: EncoderToolFlags {
                ref_frame_mvs: true,
                ..EncoderToolFlags::default()
            },
        };
        let seq = build_encoder_sequence_header(&stream).unwrap();
        let view = CoreSeqView::from_sequence(&seq).unwrap();
        for refs in [1, 2] {
            let mut slots = Slots::after_key(64, 64, 4);
            slots.valid[1] = refs == 2;
            slots.hints[1] = 2;
            slots.counters[1] = 1;
            let params = InterFrameParams {
                base_q_idx: 90,
                order_hint: 5,
                refresh_frame_flags: 0b100,
                show_frame: true,
                primary_slot: Some(0),
                use_ref_frame_mvs: true,
                ..InterFrameParams::default()
            };
            let header = build_inter_frame_header(&view, &slots.view(), &params).unwrap();
            let inter = header.core.inter.as_ref().unwrap();
            assert_eq!(inter.num_total_refs, Some(refs));
            assert_eq!(inter.use_ref_frame_mvs, Some(true));
            assert_eq!(inter.primary_ref_frame, Some(0));
            let step = refs == 2 && sb == SuperblockSize::Block128x128;
            assert_eq!(inter.tmvp_sample_step_minus_1, step.then_some(false));
            assert_eq!(
                header.core.consumed_bits,
                header.bits.num_frame_header_bits()
            );
        }
        stream.tools.ref_frame_mvs = false;
        let seq = build_encoder_sequence_header(&stream).unwrap();
        let view = CoreSeqView::from_sequence(&seq).unwrap();
        let params = InterFrameParams {
            base_q_idx: 90,
            order_hint: 5,
            show_frame: true,
            use_ref_frame_mvs: true,
            ..InterFrameParams::default()
        };
        let slots = Slots::after_key(64, 64, 4);
        assert!(build_inter_frame_header(&view, &slots.view(), &params).is_err());
    }
}

#[test]
fn inter_obu_carries_the_recorded_header_and_tile() {
    let seq = sequence(130, 66, BitDepthIdc::Ten, SuperblockSize::Block64x64);
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let slots = Slots::after_key(130, 66, 0);
    let header = build_inter_frame_header(
        &view,
        &slots.view(),
        &InterFrameParams {
            base_q_idx: 120,
            order_hint: 1,
            refresh_frame_flags: 1,
            show_frame: true,
            filters: filters(SuperblockSize::Block64x64),
            tx: KeyFrameTxParams::default(),
            tiles: FrameTiles::default(),
            reference_select: true,
            skip_mode_present: false,
            primary_slot: Some(0),
            use_ref_frame_mvs: false,
            high_precision_mv: false,
            switchable_interp_filter: false,
            tip_weight: None,
            tip_global_mv: TipGlobalMv::default(),
            tip_filter: None,
            tip_deblocking: false,
            tip_hole_fill: false,
            opfl_refine_type: 0,
            force_integer_mv: false,
            warp: false,
            global_motion: [None; 7],
            global_motion_predict: false,
            explicit_refs: None,
            implicit_output: false,
            leading: false,
        },
    )
    .unwrap();
    let tile = [0xab, 0xcd, 0xef];
    let obu = encode_inter_frame_obu(&header, &[&tile], 1).unwrap();
    let parsed = parse_annex_b_obus_partial(&obu);
    assert!(parsed.error.is_none());
    assert_eq!(parsed.obus.len(), 1);
    let frame = &parsed.obus[0];
    assert_eq!(frame.header.obu_type, ObuType::RegularTileGroup);
    assert!(frame.payload.ends_with(&tile));
    let mut reader = BitReader::new(frame.payload, ByteOffset::new(0));
    assert_eq!(reader.read_bit().unwrap(), 1, "is_first_tile_group");
    let input = FrameHeaderParseInput {
        obu_type: ObuType::RegularTileGroup,
        first_picture_in_tu: true,
        active_sequence: Some(&seq),
        mfh_record: None,
        reference_state: slots.view(),
    };
    assert_eq!(
        parse_frame_header_core(&mut reader, &input).unwrap(),
        header.core
    );
}

/// Tiles split into groups of consecutive tiles in raster order whose sizes
/// differ by at most 1 (§ 5.19); the count clamps to `1..=num_tiles`.
#[test]
fn tile_group_ranges_split_the_tiles_evenly() {
    assert_eq!(tile_group_ranges(4, 1), [(0, 3)]);
    assert_eq!(tile_group_ranges(4, 3), [(0, 0), (1, 1), (2, 3)]);
    assert_eq!(tile_group_ranges(5, 2), [(0, 1), (2, 4)]);
    assert_eq!(tile_group_ranges(4, 8), [(0, 0), (1, 1), (2, 2), (3, 3)]);
    assert_eq!(tile_group_ranges(1, 0), [(0, 0)]);
}

#[test]
fn show_existing_frame_obu_parses_complete() {
    let seq = sequence(17, 9, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let obu = encode_show_existing_frame_obu(&view, 5, None, None, (false, false)).unwrap();
    let parsed = parse_annex_b_obus_partial(&obu);
    assert!(parsed.error.is_none());
    assert_eq!(parsed.obus.len(), 1);
    let sef = &parsed.obus[0];
    assert_eq!(sef.header.obu_type, ObuType::RegularSef);
    let slots = Slots::after_key(17, 9, 7);
    let input = FrameHeaderParseInput {
        obu_type: ObuType::RegularSef,
        first_picture_in_tu: true,
        active_sequence: Some(&seq),
        mfh_record: None,
        reference_state: slots.view(),
    };
    let core =
        parse_frame_header_core(&mut BitReader::new(sef.payload, ByteOffset::new(0)), &input)
            .unwrap();
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::ShowExistingFrameComplete
    );
    assert_eq!(core.frame_to_show_map_idx, Some(5));
    assert_eq!(core.order_hint, Some(7));
    assert!(encode_show_existing_frame_obu(&view, 8, None, None, (false, false)).is_err());
}

/// The multi-frame header OBU parses to `mfh_id 1` of sequence header 0 with
/// no frame size, deblocking update or segmentation (§ 5.7), and its
/// truncated payload is an EOF error. Key and show-existing frames that
/// refer to it code `cur_mfh_id = 1` without `seq_header_id_in_frame_header`
/// (§ 5.18.2).
#[test]
fn multi_frame_header_obu_and_references_parse_back() {
    let obu = encode_multi_frame_header_obu().unwrap();
    let parsed = parse_annex_b_obus_partial(&obu);
    assert!(parsed.error.is_none());
    assert_eq!(parsed.obus.len(), 1);
    assert_eq!(parsed.obus[0].header.obu_type, ObuType::MultiFrameHeader);
    let payload = parsed.obus[0].payload;
    let mfh =
        crate::hls::parse_multi_frame_header(&mut BitReader::new(payload, ByteOffset::new(0)))
            .unwrap();
    assert_eq!((mfh.mfh_id(), mfh.mfh_seq_header_id), (1, 0));
    assert!(mfh.mfh_frame_size.is_none());
    assert!(!mfh.mfh_deblocking_filter_update && !mfh.mfh_seg_info_present_flag);
    let short = &payload[..payload.len() - 1];
    assert!(
        crate::hls::parse_multi_frame_header(&mut BitReader::new(short, ByteOffset::new(0)))
            .is_err()
    );
    let seq = sequence(17, 9, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    for multi_frame_header in [false, true] {
        let tx = KeyFrameTxParams {
            multi_frame_header,
            ..KeyFrameTxParams::default()
        };
        let core = crate::headers::frame::build_key_frame_header(
            &view,
            90,
            0,
            &FrameFilterParams::default(),
            tx,
            FrameTiles::default(),
        )
        .unwrap();
        assert_eq!(core.cur_mfh_id.get(), u32::from(multi_frame_header));
        assert_eq!(
            core.seq_header_id_in_frame_header.is_none(),
            multi_frame_header
        );
        let sef = encode_show_existing_frame_obu(&view, 5, None, None, (multi_frame_header, false))
            .unwrap();
        let sef = parse_annex_b_obus_partial(&sef).obus[0].payload;
        let mut reader = BitReader::new(sef, ByteOffset::new(0));
        assert_eq!(reader.read_uvlc().unwrap(), u32::from(multi_frame_header));
    }
}

/// An explicit `sef_order_hint` codes `derive_sef_order_hint = 0` and its
/// low `OrderHintBits` (§ 5.18.2); the parser rebuilds the display order
/// hint, and a payload cut inside the hint does not parse.
#[test]
fn show_existing_frame_obu_codes_an_explicit_order_hint() {
    let seq = sequence(17, 9, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let slots = Slots::after_key(17, 9, 7);
    let input = FrameHeaderParseInput {
        obu_type: ObuType::RegularSef,
        first_picture_in_tu: true,
        active_sequence: Some(&seq),
        mfh_record: None,
        reference_state: slots.view(),
    };
    let obu =
        encode_show_existing_frame_obu(&view, 5, Some(9 + 256), None, (false, false)).unwrap();
    let parsed = parse_annex_b_obus_partial(&obu);
    let payload = parsed.obus[0].payload;
    let core =
        parse_frame_header_core(&mut BitReader::new(payload, ByteOffset::new(0)), &input).unwrap();
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::ShowExistingFrameComplete
    );
    assert_eq!(core.derive_sef_order_hint, Some(false));
    assert_eq!((core.order_hint_lsb, core.order_hint), (Some(9), Some(9)));
    let cut = &payload[..1];
    assert!(parse_frame_header_core(&mut BitReader::new(cut, ByteOffset::new(0)), &input).is_err());
}

/// A film-grain sequence codes `apply_grain` with the seed of each output
/// key or show-existing frame, refuses a seed without film grain, refuses
/// models that break § 6.17.10.2, and its grain OBU parses back to the model.
#[test]
fn film_grain_headers_code_the_seed_and_the_model() {
    let mut params = EncoderStreamParams {
        width: 17,
        height: 9,
        bit_depth: BitDepthIdc::Eight,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: SuperblockSize::Block64x64,
        tools: EncoderToolFlags {
            film_grain: true,
            ..EncoderToolFlags::default()
        },
    };
    let seq = build_encoder_sequence_header(&params).unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    assert_eq!(view.film_grain_params_present, Some(true));
    let tx = KeyFrameTxParams {
        grain_seed: Some(0xa5c3),
        ..KeyFrameTxParams::default()
    };
    let core = crate::headers::frame::build_key_frame_header(
        &view,
        90,
        0,
        &FrameFilterParams::default(),
        tx,
        FrameTiles::default(),
    )
    .unwrap();
    let grain = core.intra_tail.unwrap().film_grain;
    assert_eq!(
        (grain.apply_grain, grain.fgm_id, grain.grain_seed),
        (true, Some(0), Some(0xa5c3))
    );
    let sef = encode_show_existing_frame_obu(&view, 2, None, Some(0x0f0f), (false, false)).unwrap();
    assert!(parse_annex_b_obus_partial(&sef).error.is_none());
    params.tools.film_grain = false;
    let plain =
        CoreSeqView::from_sequence(&build_encoder_sequence_header(&params).unwrap()).unwrap();
    assert!(encode_show_existing_frame_obu(&plain, 2, None, Some(1), (false, false)).is_err());
    assert!(
        crate::headers::frame::build_key_frame_header(
            &plain,
            90,
            0,
            &FrameFilterParams::default(),
            tx,
            FrameTiles::default()
        )
        .is_err()
    );
    let points = GrainScaling::new(vec![(16, 40), (112, 64), (240, 9)]).unwrap();
    let cb = GrainScaling::new(vec![(128, 20)]).unwrap();
    let none = GrainScaling::default();
    assert!(GrainScaling::new(vec![(16, 1), (16, 2)]).is_err());
    assert!(GrainScaling::new((0..15).map(|v| (v, 1)).collect()).is_err());
    assert!(encoder_film_grain_model(ChromaFormatIdc::Yuv420, [&points, &cb, &none]).is_err());
    assert!(encoder_film_grain_model(ChromaFormatIdc::Monochrome, [&points, &cb, &cb]).is_err());
    assert!(encoder_film_grain_model(ChromaFormatIdc::Yuv444, [&points, &cb, &none]).is_ok());
    let model = encoder_film_grain_model(ChromaFormatIdc::Yuv420, [&points, &cb, &cb]).unwrap();
    let obu = encode_film_grain_obu(ChromaFormatIdc::Yuv420, &model).unwrap();
    let parsed = parse_annex_b_obus_partial(&obu);
    assert!(parsed.error.is_none());
    let fg = crate::headers::film_grain::parse_film_grain(&mut BitReader::new(
        parsed.obus[0].payload,
        ByteOffset::new(0),
    ))
    .unwrap();
    assert_eq!((fg.update_flags, &fg.models[0].model), (1, &model));
}

/// The HDR metadata OBUs parse back as prefix short OBUs with `LAYER_GLOBAL`
/// and `GLOBAL_PERSISTENCE`; a cut OBU does not parse.
#[test]
fn hdr_metadata_obus_parse_back() {
    use crate::headers::metadata::parse_metadata_short;
    let display = MetadataHdrMdcv {
        primary_chromaticity_x: [8500, 6550, 35400],
        primary_chromaticity_y: [39850, 2300, 14600],
        white_point_chromaticity_x: 15635,
        white_point_chromaticity_y: 16450,
        luminance_max: 10_000_000,
        luminance_min: 1,
    };
    let light = MetadataHdrCll {
        max_cll: u16::MAX,
        max_fall: 0,
    };
    for (obu, payload) in [
        (
            encode_hdr_mdcv_obu(display).unwrap(),
            MetadataPayload::HdrMdcv(display),
        ),
        (
            encode_hdr_cll_obu(light).unwrap(),
            MetadataPayload::HdrCll(light),
        ),
    ] {
        let parsed = parse_annex_b_obus_partial(&obu);
        assert!(parsed.error.is_none());
        let [envelope] = &parsed.obus[..] else {
            unreachable!("one OBU");
        };
        assert_eq!(envelope.header.obu_type, ObuType::MetadataShort);
        let parse = |bytes: &[u8]| {
            parse_metadata_short(&mut BitReader::new(bytes, ByteOffset::new(0)), bytes.len())
        };
        let short = parse(envelope.payload).unwrap();
        assert!(!short.metadata_is_suffix && !short.muh_cancel_flag);
        assert_eq!((short.muh_layer_idc, short.muh_persistence_idc), (1, 0));
        assert_eq!(short.unit.unwrap().payload, payload);
        assert!(parse(&envelope.payload[..envelope.payload.len() - 2]).is_err());
    }
}

#[test]
fn rejects_out_of_range_inter_inputs() {
    let seq = sequence(64, 64, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let slots = Slots::after_key(64, 64, 0);
    let params = InterFrameParams {
        base_q_idx: 0,
        order_hint: 1,
        refresh_frame_flags: 1,
        show_frame: true,
        filters: FrameFilterParams::default(),
        tx: KeyFrameTxParams::default(),
        tiles: FrameTiles::default(),
        reference_select: false,
        skip_mode_present: false,
        primary_slot: None,
        use_ref_frame_mvs: false,
        high_precision_mv: false,
        switchable_interp_filter: false,
        tip_weight: None,
        tip_global_mv: TipGlobalMv::default(),
        tip_filter: None,
        tip_deblocking: false,
        tip_hole_fill: false,
        opfl_refine_type: 0,
        force_integer_mv: false,
        warp: false,
        global_motion: [None; 7],
        global_motion_predict: false,
        explicit_refs: None,
        implicit_output: false,
        leading: false,
    };
    assert!(matches!(
        build_inter_frame_header(&view, &slots.view(), &params),
        Err(EncoderHeaderError::InvalidTxParams { .. })
    ));
    let no_valid_slot = Slots {
        valid: [false; 8],
        ..Slots::after_key(64, 64, 0)
    };
    let params = InterFrameParams {
        base_q_idx: 60,
        ..params
    };
    assert!(build_inter_frame_header(&view, &no_valid_slot.view(), &params).is_err());
    let integer_mv = InterFrameParams {
        force_integer_mv: true,
        warp: false,
        global_motion: [None; 7],
        ..params
    };
    let slots = Slots::after_key(64, 64, 0);
    assert!(build_inter_frame_header(&view, &slots.view(), &integer_mv).is_err());
}

/// A TIP output header parses complete, needs TIP output, two references
/// and a weight, and codes `apply_grain` with the seed only in a film-grain
/// sequence (§ 5.18.2, § 5.18.10.1).
#[test]
fn tip_output_header_parses_complete_and_needs_tip_output() {
    let mut stream = EncoderStreamParams {
        width: 64,
        height: 64,
        bit_depth: BitDepthIdc::Eight,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: SuperblockSize::Block128x128,
        tools: EncoderToolFlags {
            ref_frame_mvs: true,
            tip: true,
            tip_output: true,
            ..EncoderToolFlags::default()
        },
    };
    let mut slots = Slots::after_key(64, 64, 4);
    slots.valid[1] = true;
    slots.hints[1] = 8;
    slots.counters[1] = 1;
    let params = InterFrameParams {
        order_hint: 6,
        show_frame: true,
        tip_weight: Some(3),
        tip_hole_fill: false,
        ..InterFrameParams::default()
    };
    let seq = build_encoder_sequence_header(&stream).unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let header = build_tip_output_frame_header(&view, &slots.view(), &params, &[0, 1]).unwrap();
    assert_eq!(header.core.obu_type, ObuType::RegularTip);
    let inter = header.core.inter.as_ref().unwrap();
    assert_eq!(inter.tip_frame_mode, Some(TipFrameMode::AsOutput));
    assert_eq!(inter.tip_global_wtd_index, Some(3));
    assert_eq!(
        inter.tip_interpolation_filter,
        Some(InterpolationFilter::EighttapSharp)
    );
    assert_eq!(header.core.refresh_frame_flags, Some(0));
    let obu = encode_tip_output_frame_obu(&header).unwrap();
    let parsed = parse_annex_b_obus_partial(&obu);
    assert!(parsed.error.is_none());
    assert_eq!(parsed.obus.len(), 1);
    assert_eq!(parsed.obus[0].header.obu_type, ObuType::RegularTip);
    let no_weight = InterFrameParams {
        tip_weight: None,
        tip_hole_fill: false,
        opfl_refine_type: 0,
        ..params
    };
    assert!(build_tip_output_frame_header(&view, &slots.view(), &no_weight, &[0, 1]).is_err());
    assert!(build_tip_output_frame_header(&view, &slots.view(), &params, &[0]).is_err());
    let seeded = InterFrameParams {
        tx: KeyFrameTxParams {
            grain_seed: Some(0x5a3c),
            ..KeyFrameTxParams::default()
        },
        ..params
    };
    assert!(build_tip_output_frame_header(&view, &slots.view(), &seeded, &[0, 1]).is_err());
    stream.tools.film_grain = true;
    let grain_view =
        CoreSeqView::from_sequence(&build_encoder_sequence_header(&stream).unwrap()).unwrap();
    for (params, expected) in [
        (seeded, (true, Some(0), Some(0x5a3c))),
        (params, (false, None, None)),
    ] {
        let header =
            build_tip_output_frame_header(&grain_view, &slots.view(), &params, &[0, 1]).unwrap();
        let grain = header.core.inter.unwrap().tip_film_grain.unwrap();
        assert_eq!(
            (grain.apply_grain, grain.fgm_id, grain.grain_seed),
            expected
        );
    }
    stream.tools.film_grain = false;
    stream.tools.tip_output = false;
    let seq = build_encoder_sequence_header(&stream).unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    assert!(build_tip_output_frame_header(&view, &slots.view(), &params, &[0, 1]).is_err());
}

/// The TIP output options parse back: a global MV, each TIP filter,
/// explicit quantization and TIP deblocking; out-of-range values are
/// rejected before any bit is written, and a header cut inside the options
/// does not parse complete.
#[test]
fn tip_output_options_round_trip() {
    let stream = |explicit_qp| EncoderStreamParams {
        width: 64,
        height: 64,
        bit_depth: BitDepthIdc::Eight,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: SuperblockSize::Block128x128,
        tools: EncoderToolFlags {
            ref_frame_mvs: true,
            tip: true,
            tip_output: true,
            df_sub_pu: true,
            tip_explicit_qp: explicit_qp,
            ..EncoderToolFlags::default()
        },
    };
    let mut slots = Slots::after_key(64, 64, 4);
    slots.valid[1] = true;
    slots.hints[1] = 8;
    slots.counters[1] = 1;
    let base = InterFrameParams {
        base_q_idx: 120,
        order_hint: 6,
        show_frame: true,
        tip_weight: Some(3),
        ..InterFrameParams::default()
    };
    let filters = [
        (None, InterpolationFilter::EighttapSharp),
        (
            Some(InterpolationFilter::Eighttap),
            InterpolationFilter::Eighttap,
        ),
        (
            Some(InterpolationFilter::EighttapSmooth),
            InterpolationFilter::EighttapSmooth,
        ),
    ];
    for explicit_qp in [false, true] {
        let seq = build_encoder_sequence_header(&stream(explicit_qp)).unwrap();
        let view = CoreSeqView::from_sequence(&seq).unwrap();
        for (mv, (filter, parsed), deblocking) in [
            (TipGlobalMv::new(3, -2), filters[0], false),
            (TipGlobalMv::new(0, 15), filters[1], true),
            (TipGlobalMv::new(-15, 0), filters[2], false),
            (TipGlobalMv::default(), filters[1], true),
        ] {
            let params = InterFrameParams {
                tip_global_mv: mv,
                tip_filter: filter,
                tip_deblocking: deblocking,
                ..base
            };
            let header =
                build_tip_output_frame_header(&view, &slots.view(), &params, &[0, 1]).unwrap();
            let inter = header.core.inter.as_ref().unwrap();
            assert_eq!(inter.tip_global_mv, Some(mv));
            assert_eq!(inter.tip_interpolation_filter, Some(parsed));
            assert_eq!(inter.allow_df_sub_pu, Some(deblocking));
            assert_eq!(inter.apply_deblocking_filter_tip, Some(deblocking));
            assert_eq!(header.core.tile_info.is_some(), deblocking);
            let qp = header.core.quantization_params.map(|q| q.base_q_idx);
            assert_eq!(qp, explicit_qp.then_some(120));
            let total = header.core.consumed_bits;
            for cut in [total / 16 * 8, (total - 1) / 8 * 8] {
                let mut bits = BitWriter::new();
                for index in 0..cut {
                    bits.write_flag(header.bits.bit(index).unwrap()).unwrap();
                }
                let bytes = bits.into_bytes();
                let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
                let prefix =
                    parse_frame_header_prefix(&mut reader, ObuType::RegularTip, Some(true));
                let complete = prefix.is_ok_and(|prefix| {
                    let mut core = init_core_from_prefix(&prefix, ObuType::RegularTip, true);
                    parse_core_body(&mut reader, &mut core, &view, None, &slots.view()).is_ok()
                        && core.status == FrameHeaderParseStatus::InterHeaderComplete
                        && reader.consumed_bits() == header.core.consumed_bits
                });
                assert!(!complete, "cut at {cut}");
            }
        }
    }
    let seq = build_encoder_sequence_header(&stream(false)).unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    for bad in [
        InterFrameParams {
            tip_global_mv: TipGlobalMv::new(16, 0),
            ..base
        },
        InterFrameParams {
            tip_global_mv: TipGlobalMv::new(0, i32::MIN),
            ..base
        },
        InterFrameParams {
            tip_filter: Some(InterpolationFilter::Bilinear),
            ..base
        },
    ] {
        assert!(build_tip_output_frame_header(&view, &slots.view(), &bad, &[0, 1]).is_err());
    }
    let mut plain = stream(false);
    plain.tools.df_sub_pu = false;
    let seq = build_encoder_sequence_header(&plain).unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let deblocked = InterFrameParams {
        tip_deblocking: true,
        ..base
    };
    assert!(build_tip_output_frame_header(&view, &slots.view(), &deblocked, &[0, 1]).is_err());
}

#[test]
fn opfl_refine_type_parses_with_and_without_tip() {
    for tip in [false, true] {
        let stream = EncoderStreamParams {
            width: 64,
            height: 64,
            bit_depth: BitDepthIdc::Eight,
            chroma: ChromaFormatIdc::Yuv420,
            sb_size: SuperblockSize::Block64x64,
            tools: EncoderToolFlags {
                ref_frame_mvs: true,
                tip,
                opfl_refine: true,
                refinemv: true,
                ..EncoderToolFlags::default()
            },
        };
        let seq = build_encoder_sequence_header(&stream).unwrap();
        let inter = seq.inter.as_ref().unwrap();
        assert!(
            inter.enable_refinemv && inter.enable_opfl_refine == 3 && !inter.enable_tip_refinemv
        );
        let view = CoreSeqView::from_sequence(&seq).unwrap();
        let mut slots = Slots::after_key(64, 64, 4);
        slots.valid[1] = true;
        slots.hints[1] = 8;
        slots.counters[1] = 1;
        for refine in 0..3 {
            let params = InterFrameParams {
                base_q_idx: 90,
                order_hint: 6,
                show_frame: true,
                use_ref_frame_mvs: true,
                tip_weight: tip.then_some(0),
                tip_hole_fill: false,
                opfl_refine_type: refine,
                ..InterFrameParams::default()
            };
            let header = build_inter_frame_header(&view, &slots.view(), &params).unwrap();
            let inter = header.core.inter.as_ref().unwrap();
            assert_eq!(inter.opfl_refine_type, Some(u32::from(refine)), "tip {tip}");
        }
        let bad = InterFrameParams {
            base_q_idx: 90,
            order_hint: 6,
            show_frame: true,
            opfl_refine_type: 3,
            ..InterFrameParams::default()
        };
        assert!(build_inter_frame_header(&view, &slots.view(), &bad).is_err());
    }
}

#[test]
fn tip_weight_follows_tip_refinement() {
    for tip_refinemv in [false, true] {
        let stream = EncoderStreamParams {
            width: 64,
            height: 64,
            bit_depth: BitDepthIdc::Eight,
            chroma: ChromaFormatIdc::Yuv420,
            sb_size: SuperblockSize::Block64x64,
            tools: EncoderToolFlags {
                ref_frame_mvs: true,
                tip: true,
                tip_output: true,
                opfl_refine: true,
                tip_refinemv,
                ..EncoderToolFlags::default()
            },
        };
        let seq = build_encoder_sequence_header(&stream).unwrap();
        assert_eq!(
            seq.inter.as_ref().unwrap().enable_tip_refinemv,
            tip_refinemv
        );
        let view = CoreSeqView::from_sequence(&seq).unwrap();
        let mut slots = Slots::after_key(64, 64, 4);
        slots.valid[1] = true;
        slots.hints[1] = 8;
        slots.counters[1] = 1;
        let params = |weight| InterFrameParams {
            base_q_idx: 90,
            order_hint: 6,
            show_frame: true,
            use_ref_frame_mvs: true,
            tip_weight: Some(weight),
            tip_hole_fill: false,
            opfl_refine_type: 1,
            ..InterFrameParams::default()
        };
        let header = build_inter_frame_header(&view, &slots.view(), &params(0)).unwrap();
        let inter = header.core.inter.as_ref().unwrap();
        assert_eq!(inter.tip_global_wtd_index, Some(0));
        assert_eq!(
            build_inter_frame_header(&view, &slots.view(), &params(3)).is_ok(),
            !tip_refinemv
        );
        let output = InterFrameParams {
            refresh_frame_flags: 0,
            ..params(0)
        };
        let header = build_tip_output_frame_header(&view, &slots.view(), &output, &[0, 1]).unwrap();
        assert_eq!(
            header.core.inter.as_ref().unwrap().tip_frame_mode,
            Some(TipFrameMode::AsOutput)
        );
        let weighted = InterFrameParams {
            tip_weight: Some(3),
            tip_hole_fill: false,
            ..output
        };
        assert_eq!(
            build_tip_output_frame_header(&view, &slots.view(), &weighted, &[0, 1]).is_ok(),
            !tip_refinemv
        );
    }
}

#[test]
fn warp_frames_code_their_motion_modes_and_allow_warpmv_mode() {
    let stream = EncoderStreamParams {
        width: 64,
        height: 64,
        bit_depth: BitDepthIdc::Eight,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: SuperblockSize::Block64x64,
        tools: EncoderToolFlags {
            warp: true,
            ..EncoderToolFlags::default()
        },
    };
    let seq = build_encoder_sequence_header(&stream).unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let slots = Slots::after_key(64, 64, 4);
    for warp in [false, true] {
        let params = InterFrameParams {
            base_q_idx: 90,
            order_hint: 5,
            show_frame: true,
            warp,
            ..InterFrameParams::default()
        };
        let header = build_inter_frame_header(&view, &slots.view(), &params).unwrap();
        let modes = header
            .core
            .inter
            .as_ref()
            .unwrap()
            .frame_enabled_motion_modes;
        assert_eq!(modes, Some([false, false, warp, warp, warp]));
        assert_eq!(header.core.inter_tail.unwrap().allow_warpmv_mode, warp);
    }
    let plain = CoreSeqView::from_sequence(&sequence(
        64,
        64,
        BitDepthIdc::Eight,
        SuperblockSize::Block64x64,
    ))
    .unwrap();
    let params = InterFrameParams {
        base_q_idx: 90,
        order_hint: 5,
        show_frame: true,
        warp: true,
        ..InterFrameParams::default()
    };
    assert!(build_inter_frame_header(&plain, &slots.view(), &params).is_err());
}

#[test]
fn global_motion_models_round_trip_through_the_parser() {
    let stream = EncoderStreamParams {
        width: 64,
        height: 64,
        bit_depth: BitDepthIdc::Eight,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: SuperblockSize::Block64x64,
        tools: EncoderToolFlags {
            global_motion: true,
            ..EncoderToolFlags::default()
        },
    };
    let seq = build_encoder_sequence_header(&stream).unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let mut slots = Slots::after_key(64, 64, 4);
    slots.valid[1] = true;
    slots.hints[1] = 8;
    slots.counters[1] = 1;
    let rotzoom = [
        3 << 13,
        -5 << 13,
        (1 << 16) + (17 << 6),
        -9 << 6,
        9 << 6,
        (1 << 16) + (17 << 6),
    ];
    let affine = [
        -(2 << 13),
        1 << 13,
        (1 << 16) - (3 << 6),
        4 << 6,
        -(5 << 6),
        (1 << 16) + (6 << 6),
    ];
    for models in [
        [None; 7],
        [Some(rotzoom), None, None, None, None, None, None],
        {
            let mut both = [None; 7];
            (both[0], both[1]) = (Some(rotzoom), Some(affine));
            both
        },
    ] {
        let params = InterFrameParams {
            base_q_idx: 90,
            order_hint: 6,
            show_frame: true,
            global_motion: models,
            ..InterFrameParams::default()
        };
        let (counts, hints, saved) = ([0; 8], [[0; 7]; 8], [[DEFAULT_WARP_PARAMS; 7]; 8]);
        let refs = slots
            .view()
            .with_global_motion_state(&counts, &hints, &saved);
        let header = build_inter_frame_header(&view, &refs, &params).unwrap();
        let (num_total_refs, saved_hints, saved_params) = header.core.saved_global_motion_state();
        let inter = header.core.inter.as_ref().unwrap();
        assert_eq!(Some(num_total_refs), inter.num_total_refs);
        assert_eq!(
            saved_hints[..inter.order_hints.len()],
            inter.order_hints[..]
        );
        for (saved, model) in saved_params.iter().zip(models) {
            assert_eq!(*saved, model.unwrap_or(DEFAULT_WARP_PARAMS));
        }
        let gm = header.core.inter_tail.unwrap().global_motion;
        assert_eq!(gm.use_global_motion, models.iter().any(Option::is_some));
        for (reference, model) in gm.references.iter().zip(models) {
            match model {
                Some(params) => assert_eq!(reference.gm_params, params),
                None => assert_eq!(reference.gm_type, GmType::Identity),
            }
        }
    }
    let off_lattice = InterFrameParams {
        base_q_idx: 90,
        order_hint: 6,
        show_frame: true,
        global_motion: [
            Some([1, 0, 1 << 16, 0, 0, 1 << 16]),
            None,
            None,
            None,
            None,
            None,
            None,
        ],
        ..InterFrameParams::default()
    };
    let (counts, hints, saved) = ([0; 8], [[0; 7]; 8], [[DEFAULT_WARP_PARAMS; 7]; 8]);
    let refs = slots
        .view()
        .with_global_motion_state(&counts, &hints, &saved);
    assert!(build_inter_frame_header(&view, &refs, &off_lattice).is_err());
    let mut unread = [None; 7];
    unread[5] = Some([0, 0, (1 << 16) + (1 << 6), 0, 0, (1 << 16) + (1 << 6)]);
    let no_reference = InterFrameParams {
        global_motion: unread,
        ..off_lattice
    };
    assert!(build_inter_frame_header(&view, &refs, &no_reference).is_err());
}

/// The CDF controls of `KeyFrameTxParams` parse back (§ 5.18.2):
/// `disable_cdf_update` in both frame types, and in an inter frame
/// `disable_cross_frame_cdf_init` with a primary reference and
/// `PRIMARY_REF_CHOOSE`, which saves the 3 bits of `primary_ref_frame`. All off
/// code the old header; a cut header does not parse.
#[test]
fn cdf_controls_parse_back_and_cut_headers_fail() {
    let seq = sequence(64, 64, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let slots = Slots::after_key(64, 64, 0);
    let params = InterFrameParams {
        base_q_idx: 90,
        order_hint: 1,
        refresh_frame_flags: 2,
        show_frame: true,
        filters: FrameFilterParams::default(),
        tx: KeyFrameTxParams::default(),
        tiles: FrameTiles::default(),
        reference_select: false,
        skip_mode_present: false,
        primary_slot: Some(0),
        use_ref_frame_mvs: false,
        high_precision_mv: false,
        switchable_interp_filter: false,
        tip_weight: None,
        tip_hole_fill: false,
        opfl_refine_type: 0,
        force_integer_mv: false,
        warp: false,
        global_motion: [None; 7],
        global_motion_predict: false,
        explicit_refs: None,
        implicit_output: false,
        leading: false,
        tip_global_mv: TipGlobalMv::default(),
        tip_filter: None,
        tip_deblocking: false,
    };
    let with = |tx: KeyFrameTxParams| {
        build_inter_frame_header(&view, &slots.view(), &InterFrameParams { tx, ..params }).unwrap()
    };
    let off = with(KeyFrameTxParams::default());
    let controls = |header: &EncodedFrameHeader| {
        let inter = header.core.inter.as_ref().unwrap();
        (
            header.core.disable_cdf_update,
            inter.signal_primary_ref_frame,
            inter.disable_cross_frame_cdf_init,
            inter.primary_ref_frame,
        )
    };
    assert_eq!(
        controls(&off),
        (Some(false), Some(true), Some(false), Some(0))
    );
    let on = with(KeyFrameTxParams {
        disable_cdf_update: true,
        disable_cross_frame_cdf_init: true,
        ..KeyFrameTxParams::default()
    });
    assert_eq!(controls(&on), (Some(true), Some(true), Some(true), Some(0)));
    let choose = with(KeyFrameTxParams {
        choose_primary_ref_frame: true,
        ..KeyFrameTxParams::default()
    });
    assert_eq!(
        controls(&choose),
        (Some(false), Some(false), Some(false), Some(8))
    );
    assert_eq!(
        choose.bits.num_frame_header_bits() + 3,
        off.bits.num_frame_header_bits()
    );
    let key = |tx| {
        crate::headers::frame::build_key_frame_header(
            &view,
            90,
            0,
            &FrameFilterParams::default(),
            tx,
            FrameTiles::default(),
        )
        .unwrap()
    };
    let fixed = KeyFrameTxParams {
        disable_cdf_update: true,
        ..KeyFrameTxParams::default()
    };
    assert_eq!(key(fixed).disable_cdf_update, Some(true));
    assert_eq!(
        key(KeyFrameTxParams::default()).disable_cdf_update,
        Some(false)
    );
    let ignored = KeyFrameTxParams {
        disable_cross_frame_cdf_init: true,
        choose_primary_ref_frame: true,
        ..fixed
    };
    assert_eq!(key(ignored), key(fixed));
    assert_cut_header_fails(&seq, &on, &slots.view());
}

/// The inter header `header` cut by its last byte does not parse to a
/// complete header with the § 7.23 state `refs`.
fn assert_cut_header_fails(
    seq: &SequenceHeader,
    header: &EncodedFrameHeader,
    refs: &FrameReferenceStateView<'_>,
) {
    let info = header.core.tile_info.as_ref();
    let tiles =
        vec![&[0xab][..]; info.map_or(1, |info| (info.tile_cols * info.tile_rows) as usize)];
    let obu = encode_inter_frame_obu(header, &tiles, 1).unwrap();
    let payload = parse_annex_b_obus_partial(&obu).obus[0].payload;
    let cut = header.bits.num_frame_header_bits().div_ceil(8) as usize - 1;
    let input = FrameHeaderParseInput {
        obu_type: ObuType::RegularTileGroup,
        first_picture_in_tu: true,
        active_sequence: Some(seq),
        mfh_record: None,
        reference_state: *refs,
    };
    let mut reader = BitReader::new(&payload[..cut], ByteOffset::new(0));
    assert_eq!(reader.read_bit().unwrap(), 1, "is_first_tile_group");
    let cut_core = parse_frame_header_core(&mut reader, &input);
    assert!(!matches!(
        cut_core,
        Ok(ref core) if core.status == FrameHeaderParseStatus::InterHeaderComplete
    ));
}

/// A `reuse_ccso` plane of an inter frame codes its two reuse flags and no
/// parameters (§ 5.18.7.12): it parses back with the flags, `ccso_ref_idx`
/// inferred `0` over one reference, and no parameters, and the header is
/// shorter than the one that codes them. A `ccso_ref_idx` outside the
/// references and reuse in a key frame are refused; a cut header does not
/// parse complete.
#[test]
fn ccso_reuse_planes_parse_back_and_cut_headers_fail() {
    let sb = SuperblockSize::Block64x64;
    let seq = sequence(64, 64, BitDepthIdc::Eight, sb);
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let slots = Slots::after_key(64, 64, 0);
    let coded = filters(sb);
    let reuse = |ccso_ref_idx| FrameFilterParams {
        ccso: coded.ccso.map(|plane| CcsoPlaneParams {
            reuse_ccso: plane.ccso_planes,
            ccso_ref_idx: plane.ccso_planes.then_some(ccso_ref_idx),
            ..plane
        }),
        ..coded
    };
    let params = |filters| InterFrameParams {
        base_q_idx: 90,
        order_hint: 1,
        refresh_frame_flags: 2,
        show_frame: true,
        filters,
        tx: KeyFrameTxParams::default(),
        tiles: FrameTiles::default(),
        reference_select: false,
        skip_mode_present: false,
        primary_slot: Some(0),
        use_ref_frame_mvs: false,
        high_precision_mv: false,
        switchable_interp_filter: false,
        tip_weight: None,
        tip_hole_fill: false,
        opfl_refine_type: 0,
        force_integer_mv: false,
        warp: false,
        global_motion: [None; 7],
        tip_global_mv: TipGlobalMv::default(),
        tip_filter: None,
        tip_deblocking: false,
        global_motion_predict: false,
        explicit_refs: None,
        implicit_output: false,
        leading: false,
    };
    let build = |filters| build_inter_frame_header(&view, &slots.view(), &params(filters));
    let fresh = build(coded).unwrap();
    let reused = build(reuse(0)).unwrap();
    let plane = &reused.core.ccso_params.as_ref().unwrap().planes[1];
    assert!(plane.ccso_planes && plane.reuse_ccso && !plane.sb_reuse_ccso);
    assert_eq!(plane.ccso_ref_idx, Some(0));
    assert_eq!(plane.ccso_scale_idx, None);
    assert!(reused.bits.num_frame_header_bits() < fresh.bits.num_frame_header_bits());
    let fresh_plane = &fresh.core.ccso_params.as_ref().unwrap().planes[1];
    assert!(!fresh_plane.reuse_ccso && fresh_plane.ccso_scale_idx == Some(2));
    assert!(build(reuse(1)).is_err(), "one reference has no index 1");
    let key = crate::headers::frame::build_key_frame_header(
        &view,
        90,
        0,
        &reuse(0),
        KeyFrameTxParams::default(),
        FrameTiles::default(),
    );
    assert!(key.is_err(), "a key frame cannot reuse CCSO");
    let obu = encode_inter_frame_obu(&reused, &[&[0xab]], 1).unwrap();
    let payload = parse_annex_b_obus_partial(&obu).obus[0].payload;
    let cut = reused.bits.num_frame_header_bits().div_ceil(8) as usize - 1;
    let input = FrameHeaderParseInput {
        obu_type: ObuType::RegularTileGroup,
        first_picture_in_tu: true,
        active_sequence: Some(&seq),
        mfh_record: None,
        reference_state: slots.view(),
    };
    let mut reader = BitReader::new(&payload[..cut], ByteOffset::new(0));
    assert_eq!(reader.read_bit().unwrap(), 1, "is_first_tile_group");
    let cut_core = parse_frame_header_core(&mut reader, &input);
    assert!(!matches!(
        cut_core,
        Ok(ref core) if core.status == FrameHeaderParseStatus::InterHeaderComplete
    ));
}

/// Each slot saved `model` against a frame as far behind it as the current
/// frame is ahead. With `global_motion_predict` the header codes the `our_ref`
/// and `their_ref` (§ 5.18.9.1) whose saved model predicts the coded one: fewer
/// bits and the same `gm_params`. A restricted saved order hint (§ 6.17.9.1)
/// is never chosen, so the header keeps `our_ref = NumTotalRefs`. A cut
/// header does not parse.
#[test]
fn global_motion_predict_codes_their_ref_and_the_same_models() {
    let seq = build_encoder_sequence_header(&EncoderStreamParams {
        width: 64,
        height: 64,
        bit_depth: BitDepthIdc::Eight,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: SuperblockSize::Block64x64,
        tools: EncoderToolFlags {
            global_motion: true,
            ..EncoderToolFlags::default()
        },
    })
    .unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let mut slots = Slots::after_key(64, 64, 4);
    slots.valid[1] = true;
    slots.hints[1] = 8;
    slots.counters[1] = 1;
    let model = [
        7 << 13,
        -(11 << 13),
        (1 << 16) + (40 << 6),
        -(23 << 6),
        23 << 6,
        (1 << 16) + (40 << 6),
    ];
    let mut models = [None; 7];
    models[0] = Some(model);
    let params = InterFrameParams {
        base_q_idx: 90,
        order_hint: 6,
        show_frame: true,
        global_motion: models,
        ..InterFrameParams::default()
    };
    let predict = InterFrameParams {
        global_motion_predict: true,
        ..params
    };
    let counts = [1; 8];
    let saved = [[model; 7]; 8];
    let mut hints = [[0; 7]; 8];
    for (saved_hint, &hint) in hints.iter_mut().zip(&slots.hints) {
        saved_hint[0] = 2 * hint - 6;
    }
    let refs = slots
        .view()
        .with_global_motion_state(&counts, &hints, &saved);
    let off = build_inter_frame_header(&view, &refs, &params).unwrap();
    let on = build_inter_frame_header(&view, &refs, &predict).unwrap();
    let gm = |header: &EncodedFrameHeader| header.core.inter_tail.clone().unwrap().global_motion;
    let num_total_refs = on.core.inter.as_ref().unwrap().num_total_refs;
    assert!(on.bits.num_frame_header_bits() < off.bits.num_frame_header_bits());
    assert_eq!(gm(&off).our_ref, num_total_refs);
    assert_ne!(gm(&on).our_ref, num_total_refs);
    assert_eq!(gm(&on).references, gm(&off).references);
    assert_eq!(gm(&on).references[0].gm_params, model);
    let restricted = [[u32::MAX; 7]; 8];
    let refs_restricted = slots
        .view()
        .with_global_motion_state(&counts, &restricted, &saved);
    let kept = build_inter_frame_header(&view, &refs_restricted, &predict).unwrap();
    assert_eq!(gm(&kept).our_ref, num_total_refs);
    assert_eq!(
        kept.bits,
        build_inter_frame_header(&view, &refs_restricted, &params)
            .unwrap()
            .bits
    );
    assert_cut_header_fails(&seq, &on, &refs);
}

/// With `enable_short_refresh_frame_flags` an inter or TIP output header
/// codes `has_refresh_frame_flags` and the `frame_to_refresh` of its one
/// slot (§ 5.18.2), parses back to the same `refresh_frame_flags` and is
/// shorter than the `NumRefFrames` bits; two slots, or a slot outside
/// `NumRefFrames`, are rejected; a cut header does not parse.
#[test]
fn short_refresh_flags_code_one_slot_or_none() {
    let stream = |short_refresh| EncoderStreamParams {
        width: 64,
        height: 64,
        bit_depth: BitDepthIdc::Eight,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: SuperblockSize::Block64x64,
        tools: EncoderToolFlags {
            ref_frame_mvs: true,
            tip: true,
            tip_output: true,
            short_refresh,
            ..EncoderToolFlags::default()
        },
    };
    let seqs = [false, true].map(|short| build_encoder_sequence_header(&stream(short)).unwrap());
    let [long, short] = [0, 1].map(|i| CoreSeqView::from_sequence(&seqs[i]).unwrap());
    assert!(!long.enable_short_refresh_frame_flags && short.enable_short_refresh_frame_flags);
    let mut slots = Slots::after_key(64, 64, 4);
    slots.valid[1] = true;
    slots.hints[1] = 8;
    slots.counters[1] = 1;
    let params = |refresh_frame_flags| InterFrameParams {
        base_q_idx: 90,
        order_hint: 6,
        refresh_frame_flags,
        show_frame: true,
        primary_slot: Some(0),
        tip_weight: Some(3),
        ..InterFrameParams::default()
    };
    for (flags, saved) in [(0, 7), (0b10, 4), (0b1000_0000, 4)] {
        let bits = |view: &CoreSeqView| {
            let header = build_inter_frame_header(view, &slots.view(), &params(flags)).unwrap();
            assert_eq!(header.core.refresh_frame_flags, Some(flags), "{flags:#b}");
            assert_eq!(
                header.core.consumed_bits,
                header.bits.num_frame_header_bits()
            );
            header
        };
        let header = bits(&short);
        let long_bits = bits(&long).bits.num_frame_header_bits();
        assert_eq!(long_bits - header.bits.num_frame_header_bits(), saved);
        assert_cut_header_fails(&seqs[1], &header, &slots.view());
    }
    for flags in [0b11, 1 << 8] {
        assert!(build_inter_frame_header(&short, &slots.view(), &params(flags)).is_err());
    }
    let tip = build_tip_output_frame_header(&short, &slots.view(), &params(0), &[0, 1]).unwrap();
    assert_eq!(tip.core.refresh_frame_flags, Some(0));
    let obu = encode_tip_output_frame_obu(&tip).unwrap();
    let parsed = parse_annex_b_obus_partial(&obu);
    assert!(parsed.error.is_none() && parsed.obus.len() == 1);
}

/// A sequence with `seq_tile_info_present_flag` codes its uniform layout
/// (§ 5.4.2); key and inter frames reuse it when `allow_tile_info_change` is
/// off, code `reuse_tile_info` when it is on, and code their own tile info
/// after `reuse_tile_info = 0`. A frame that reuses the layout codes it in
/// place of its own; a bad `TileSizeBytes` is rejected; a cut header does not
/// parse.
#[test]
fn sequence_tile_info_is_reused_or_coded_per_frame() {
    for (allow, reuse, saved) in [(false, true, 3), (true, true, 2), (true, false, -1)] {
        let tiles = |cols_log2| FrameTiles {
            cols_log2,
            rows_log2: 0,
            reuse_tile_info: reuse,
            ..FrameTiles::default()
        };
        let stream = |seq_tiles| EncoderStreamParams {
            width: 256,
            height: 64,
            bit_depth: BitDepthIdc::Eight,
            chroma: ChromaFormatIdc::Yuv420,
            sb_size: SuperblockSize::Block64x64,
            tools: EncoderToolFlags {
                seq_tiles,
                ..EncoderToolFlags::default()
            },
        };
        let present = Some(SeqTiles {
            cols_log2: 1,
            rows_log2: 0,
            allow_tile_info_change: allow,
        });
        let seqs = [None, present]
            .map(|seq_tiles| build_encoder_sequence_header(&stream(seq_tiles)).unwrap());
        let [plain, seq] = [0, 1].map(|i| CoreSeqView::from_sequence(&seqs[i]).unwrap());
        assert!(seq.tile.seq_tile_info_present_flag && seq.tile.allow_tile_info_change == allow);
        assert_eq!(seq.tile.seq_tile_params.unwrap().tile_cols, 2);
        let slots = Slots::after_key(256, 64, 0);
        let params = |cols_log2| InterFrameParams {
            base_q_idx: 90,
            order_hint: 1,
            refresh_frame_flags: 2,
            show_frame: true,
            primary_slot: Some(0),
            tiles: tiles(cols_log2),
            ..InterFrameParams::default()
        };
        let inter = |view: &CoreSeqView, cols_log2| {
            build_inter_frame_header(view, &slots.view(), &params(cols_log2))
        };
        let key = |view: &CoreSeqView, cols_log2| {
            crate::headers::frame::build_key_frame_header(
                view,
                90,
                0,
                &FrameFilterParams::default(),
                KeyFrameTxParams::default(),
                tiles(cols_log2),
            )
        };
        let header = inter(&seq, 1).unwrap();
        let info = header.core.tile_info.as_ref().unwrap();
        assert_eq!(
            (info.tile_cols, info.reuse_tile_info),
            (2, reuse),
            "{allow} {reuse}"
        );
        let bits = |header: &EncodedFrameHeader| header.bits.num_frame_header_bits() as i64;
        assert_eq!(bits(&inter(&plain, 1).unwrap()) - bits(&header), saved);
        assert_eq!(key(&seq, 1).unwrap().tile_info.unwrap().tile_cols, 2);
        let own = inter(&seq, 0).unwrap().core.tile_info.unwrap().tile_cols;
        assert_eq!(own, if reuse { 2 } else { 1 }, "{allow} {reuse}");
        assert_eq!(key(&seq, 0).unwrap().tile_info.unwrap().tile_cols, own);
        let bad = InterFrameParams {
            tiles: FrameTiles {
                size_bytes: 5,
                ..tiles(1)
            },
            ..params(1)
        };
        assert!(build_inter_frame_header(&seq, &slots.view(), &bad).is_err());
        assert_cut_header_fails(&seqs[1], &header, &slots.view());
    }
}

/// With `explicit_ref_frame_map` an inter frame codes
/// `frame_explicit_ref_frame_map` (§ 5.18.2): `0` keeps the default map for one
/// more bit, and `1` codes `num_total_refs` and the first `ref_frame_idx` of
/// the default map. An explicit map without the sequence flag, or with no
/// reference, is rejected; a TIP output header codes the flag `0`; a cut
/// header does not parse.
#[test]
fn explicit_reference_maps_code_the_first_default_references() {
    let stream = |explicit_ref_frame_map| EncoderStreamParams {
        width: 64,
        height: 64,
        bit_depth: BitDepthIdc::Eight,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: SuperblockSize::Block64x64,
        tools: EncoderToolFlags {
            ref_frame_mvs: true,
            tip: true,
            tip_output: true,
            explicit_ref_frame_map,
            ..EncoderToolFlags::default()
        },
    };
    let seqs = [false, true].map(|map| build_encoder_sequence_header(&stream(map)).unwrap());
    let [implicit, explicit] = [0, 1].map(|i| CoreSeqView::from_sequence(&seqs[i]).unwrap());
    let mut slots = Slots::after_key(64, 64, 4);
    for slot in 1..4 {
        slots.valid[slot] = true;
        slots.hints[slot] = 4 + 2 * slot as u32;
        slots.counters[slot] = slot as u32;
    }
    let params = |explicit_refs| InterFrameParams {
        base_q_idx: 90,
        order_hint: 7,
        refresh_frame_flags: 0b1_0000,
        show_frame: true,
        primary_slot: Some(1),
        tip_weight: Some(3),
        explicit_refs,
        ..InterFrameParams::default()
    };
    let map =
        |header: &EncodedFrameHeader| header.core.inter.as_ref().unwrap().ref_frame_idx.clone();
    let default = build_inter_frame_header(&implicit, &slots.view(), &params(None)).unwrap();
    assert_eq!(default.core.inter.as_ref().unwrap().num_total_refs, Some(4));
    let bits = |header: &EncodedFrameHeader| header.bits.num_frame_header_bits();
    let flag_off = build_inter_frame_header(&explicit, &slots.view(), &params(None)).unwrap();
    assert_eq!(
        (map(&flag_off), bits(&flag_off)),
        (map(&default), bits(&default) + 1)
    );
    for limit in [1, 2, 4, 7] {
        let header =
            build_inter_frame_header(&explicit, &slots.view(), &params(Some(limit))).unwrap();
        let count = usize::from(limit).min(4);
        assert_eq!(map(&header)[..], map(&default)[..count], "{limit}");
        let inter = header.core.inter.as_ref().unwrap();
        assert_eq!(inter.explicit_ref_frame_map, Some(true));
        assert_eq!(header.core.consumed_bits, bits(&header));
        assert_cut_header_fails(&seqs[1], &header, &slots.view());
    }
    assert!(build_inter_frame_header(&implicit, &slots.view(), &params(Some(2))).is_err());
    assert!(build_inter_frame_header(&explicit, &slots.view(), &params(Some(0))).is_err());
    let tip =
        build_tip_output_frame_header(&explicit, &slots.view(), &params(None), &map(&default))
            .unwrap();
    assert_eq!(tip.core.inter.unwrap().explicit_ref_frame_map, Some(false));
}

#[test]
fn intra_only_headers_keep_the_other_slots() {
    for sb in [SuperblockSize::Block64x64, SuperblockSize::Block256x256] {
        let seq = sequence(200, 136, BitDepthIdc::Eight, sb);
        let view = CoreSeqView::from_sequence(&seq).unwrap();
        let slots = Slots::after_key(200, 136, 4);
        for (show_frame, refresh_frame_flags, filters) in [
            (true, 0b10, FrameFilterParams::default()),
            (false, 0b1000_0000, filters(sb)),
        ] {
            let params = InterFrameParams {
                base_q_idx: 90,
                order_hint: 6,
                refresh_frame_flags,
                show_frame,
                filters,
                ..InterFrameParams::default()
            };
            let header = build_intra_only_frame_header(&view, &slots.view(), &params).unwrap();
            let core = &header.core;
            assert_eq!(core.frame_type, Some(FrameType::IntraOnly));
            assert_eq!(core.frame_is_intra, Some(true));
            assert_eq!(core.refresh_frame_flags, Some(refresh_frame_flags));
            assert_eq!(core.display_order_hint(), Some(6));
            assert_eq!(core.immediate_output_frame, Some(show_frame));
            assert!(core.inter.is_none());
            assert_eq!(header.bits.num_frame_header_bits(), core.consumed_bits);
            let bytes = encode_inter_frame_obu(&header, &[&[0x5a; 3]], 1).unwrap();
            assert!(!bytes.is_empty());
            let mut w = BitWriter::new();
            for index in 0..header.bits.num_frame_header_bits() {
                w.write_flag(header.bits.bit(index).unwrap()).unwrap();
            }
            let full = w.into_bytes();
            for len in 0..core.consumed_bits.div_ceil(8) as usize {
                let mut cut = BitReader::new(&full[..len], ByteOffset::new(0));
                let parsed =
                    parse_frame_header_prefix(&mut cut, ObuType::RegularTileGroup, Some(true))
                        .and_then(|prefix| {
                            let mut cut_core =
                                init_core_from_prefix(&prefix, ObuType::RegularTileGroup, true);
                            parse_core_body(&mut cut, &mut cut_core, &view, None, &slots.view())?;
                            Ok(cut_core.status)
                        });
                let complete = FrameHeaderParseStatus::IntraHeaderComplete;
                assert!(parsed.map_or(true, |s| s != complete), "cut at {len}");
            }
        }
        let bad = |params: InterFrameParams| {
            build_intra_only_frame_header(&view, &slots.view(), &params).is_err()
        };
        let base = InterFrameParams {
            base_q_idx: 90,
            order_hint: 6,
            refresh_frame_flags: 0b10,
            ..InterFrameParams::default()
        };
        assert!(bad(InterFrameParams {
            base_q_idx: 0,
            ..base
        }));
        assert!(bad(InterFrameParams {
            tx: KeyFrameTxParams {
                tx_mode: TxMode::Only4x4,
                ..KeyFrameTxParams::default()
            },
            ..base
        }));
    }
}

/// An open-loop key frame (§ 5.18.2) is a hidden key frame that refreshes
/// the slots it names and keeps the others; its leading frames code the
/// leading tile-group and show-existing OBU types. A shown or leading OLK, a
/// zero refresh and a cut header are rejected.
#[test]
fn open_loop_key_headers_keep_the_other_slots() {
    let seq = sequence(200, 136, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let slots = Slots::after_key(200, 136, 4);
    let params = InterFrameParams {
        base_q_idx: 90,
        order_hint: 6,
        refresh_frame_flags: 0b100,
        ..InterFrameParams::default()
    };
    let header = build_open_loop_key_frame_header(&view, &slots.view(), &params).unwrap();
    let core = &header.core;
    assert_eq!(core.obu_type, ObuType::OpenLoopKey);
    assert_eq!(core.frame_type, Some(FrameType::Key));
    assert_eq!(core.refresh_frame_flags, Some(0b100));
    assert_eq!(core.display_order_hint(), Some(6));
    assert_eq!(core.immediate_output_frame, Some(false));
    assert_eq!(header.bits.num_frame_header_bits(), core.consumed_bits);
    let unit = crate::headers::frame::encode_key_frame_temporal_unit(
        &seq,
        core,
        true,
        (&[&[0x5a; 3]], 1),
        &[],
    )
    .unwrap();
    let parsed = parse_annex_b_obus_partial(&unit);
    assert!(parsed.error.is_none());
    let types: Vec<_> = parsed.obus.iter().map(|obu| obu.header.obu_type).collect();
    assert_eq!(
        types,
        [
            ObuType::TemporalDelimiter,
            ObuType::SequenceHeader,
            ObuType::OpenLoopKey
        ]
    );
    let mut reader = BitReader::new(parsed.obus[2].payload, ByteOffset::new(0));
    assert_eq!(reader.read_bit().unwrap(), 1, "is_first_tile_group");
    let input = FrameHeaderParseInput {
        obu_type: ObuType::OpenLoopKey,
        first_picture_in_tu: true,
        active_sequence: Some(&seq),
        mfh_record: None,
        reference_state: slots.view(),
    };
    assert_eq!(parse_frame_header_core(&mut reader, &input).unwrap(), *core);
    let mut w = BitWriter::new();
    for index in 0..header.bits.num_frame_header_bits() {
        w.write_flag(header.bits.bit(index).unwrap()).unwrap();
    }
    let full = w.into_bytes();
    for len in 0..core.consumed_bits.div_ceil(8) as usize {
        let mut cut = BitReader::new(&full[..len], ByteOffset::new(0));
        let parsed = parse_frame_header_prefix(&mut cut, ObuType::OpenLoopKey, Some(true))
            .and_then(|prefix| {
                let mut cut_core = init_core_from_prefix(&prefix, ObuType::OpenLoopKey, true);
                parse_core_body(&mut cut, &mut cut_core, &view, None, &slots.view())?;
                Ok(cut_core.status)
            });
        let complete = FrameHeaderParseStatus::IntraHeaderComplete;
        assert!(parsed.map_or(true, |s| s != complete), "cut at {len}");
    }
    let bad = |params: InterFrameParams| {
        build_open_loop_key_frame_header(&view, &slots.view(), &params).is_err()
    };
    assert!(bad(InterFrameParams {
        show_frame: true,
        ..params
    }));
    assert!(bad(InterFrameParams {
        leading: true,
        ..params
    }));
    assert!(bad(InterFrameParams {
        refresh_frame_flags: 0,
        ..params
    }));
    assert!(bad(InterFrameParams {
        base_q_idx: 0,
        ..params
    }));
    let leading = InterFrameParams {
        leading: true,
        refresh_frame_flags: 0b10,
        ..params
    };
    let intra = build_intra_only_frame_header(&view, &slots.view(), &leading).unwrap();
    let obu = encode_inter_frame_obu(&intra, &[&[0x5a; 3]], 1).unwrap();
    let parsed = parse_annex_b_obus_partial(&obu);
    assert_eq!(parsed.obus[0].header.obu_type, ObuType::LeadingTileGroup);
    let sef = encode_show_existing_frame_obu(&view, 2, None, None, (false, true)).unwrap();
    let parsed = parse_annex_b_obus_partial(&sef);
    assert_eq!(parsed.obus[0].header.obu_type, ObuType::LeadingSef);
}

/// With `implicit_output` the sequence codes `monotonic_output_order_flag =
/// 0`; a hidden inter frame codes `implicit_output_frame` (§ 5.18.2) and, as an
/// output frame, its film grain (§ 5.18.10.1). An implicit shown frame or an
/// implicit frame in monotonic order is rejected; a cut header does not
/// parse.
#[test]
fn hidden_frames_code_implicit_output() {
    let stream = |implicit_output| EncoderStreamParams {
        width: 64,
        height: 64,
        bit_depth: BitDepthIdc::Eight,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: SuperblockSize::Block64x64,
        tools: EncoderToolFlags {
            implicit_output,
            film_grain: true,
            ..EncoderToolFlags::default()
        },
    };
    let seqs =
        [false, true].map(|implicit| build_encoder_sequence_header(&stream(implicit)).unwrap());
    assert!(seqs[0].general.monotonic_output_order_flag);
    assert!(!seqs[1].general.monotonic_output_order_flag);
    let [monotonic, free] = [0, 1].map(|i| CoreSeqView::from_sequence(&seqs[i]).unwrap());
    let slots = Slots::after_key(64, 64, 0);
    let params = |show_frame, implicit_output, grain_seed| InterFrameParams {
        base_q_idx: 90,
        order_hint: 4,
        refresh_frame_flags: 2,
        show_frame,
        primary_slot: Some(0),
        implicit_output,
        tx: KeyFrameTxParams {
            grain_seed,
            ..KeyFrameTxParams::default()
        },
        ..InterFrameParams::default()
    };
    for (implicit, seed) in [(false, None), (true, None), (true, Some(0x1234))] {
        let header =
            build_inter_frame_header(&free, &slots.view(), &params(false, implicit, seed)).unwrap();
        assert_eq!(header.core.implicit_output_frame, Some(implicit));
        assert_eq!(
            header.core.consumed_bits,
            header.bits.num_frame_header_bits()
        );
        assert_cut_header_fails(&seqs[1], &header, &slots.view());
    }
    assert!(
        build_inter_frame_header(&free, &slots.view(), &params(false, false, Some(7))).is_err()
    );
    assert!(build_inter_frame_header(&free, &slots.view(), &params(true, true, None)).is_err());
    assert!(
        build_inter_frame_header(&monotonic, &slots.view(), &params(false, true, None)).is_err()
    );
}
