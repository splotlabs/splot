// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_parallel::ThreadCount;

use splot_core::headers::frame::{
    FrameHeaderCore, FrameHeaderParseStatus, QuantizationParams, RefIdxBuf, TipFrameMode,
};
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader};
use splot_core::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};
use splot_core::segment::{MAX_SEGMENTS, SEG_LVL_MAX, SegmentFeature};
use splot_core::span::ByteOffset;
use splot_core::stream::{
    ParsedBitstream, ParsedIvfBitstream, ParsedIvfFrame, parse_bitstream_partial,
};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameHashInput, PixelFormat, PlaneId, PlaneRect,
    PlaneSize, SharedFrame,
};

use super::block::{
    BLOCK_8X8, interp_filter_no_neighbour_ctx, tile_neighbour_availability,
    tip_allowed_for_block_indices,
};
use super::test_support::fixture_sequence_and_key_core;
use super::{
    ccso_reference_slot, compound_is_joint_context, compound_is_joint_context_from_order_hints,
    inter_segmentation_supported,
};
use crate::bitstream::tile_payload::{
    LumaCoeffBlock, reconstruct_general_intra_chroma_cctx_pair_with_predictions,
};
use crate::error::{DecodeError, DecodeHeaderStateError, Result};
use crate::pipeline::{PipelineDecodedFrame, PipelineFrame, decode_frames_from_plan};
use crate::{
    DecodeContext, DecodeLimitName, DecodeLimitThreshold, DecodeLimits, DecodeOptions,
    DecodeRuntimeConfig, DecodeStreamPlan,
};

mod header_state;
mod zero_reference;

const TWO_FRAME_INTER_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-2frame-inter-64x64.ivf");

const SEF_FAMILIES_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-frame-sef-families-64x64.ivf"
);

#[test]
fn inter_segmentation_admits_only_current_alt_q_maps() {
    let mut features = [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS];
    features[7][0] = SegmentFeature {
        enabled: true,
        data: 11,
    };
    assert!(inter_segmentation_supported(true, true, false, &features));
    assert!(!inter_segmentation_supported(true, false, false, &features));
    assert!(!inter_segmentation_supported(true, true, true, &features));
    features[7][1] = SegmentFeature {
        enabled: true,
        data: 0,
    };
    assert!(!inter_segmentation_supported(true, true, false, &features));
}

const TWO_FRAME_INTER_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-inter-64x64-10bit.ivf"
);

const DEBLOCK_INTER_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-deblock-inter-32x32-10bit-q100.ivf"
);

const CDEF_INTER_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-cdef-inter-64x32-10bit-q120.ivf"
);

const CCSO_INTER_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-ccso-inter-32x32-10bit-q100.ivf"
);

const CCSO_REUSE_INTER_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-ccso-reuse-inter-64x64-10bit.ivf"
);

const TXSPLIT_INTRA_INTER_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-txsplit-intra-inter-64x64-10bit-q100.ivf"
);

const SAMEREF_COMPOUND_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-sameref-compound-64x32-10bit-q150.ivf"
);

const SIMPLE_INTERINTRA_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-3frame-simple-interintra-64x32-10bit.ivf"
);

const FRACTIONAL_INTRABC_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-mono-intrabc-morph-128x128-q100.ivf"
);

const TWO_FRAME_SUBPEL_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-subpel-inter-64x64.ivf"
);

const TWO_FRAME_RESIDUAL_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-inter-residual-64x64.ivf"
);

const TWO_FRAME_Y_DC_DELTA_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-flatstep-inter-y-dc-delta1-64x64-q80.ivf"
);

const TWO_FRAME_MVSTACK_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-inter-mvstack-64x64.ivf"
);

const MULTI_SB_INTER_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-2sb-inter-128x64-q80.ivf");

const MULTI_TILE_INTER_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-2tile-inter-128x64-q80.ivf");

const GRID_INTER_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-grid-inter-128x128-q80.ivf");

const MVORDER_INTER_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-inter-mvorder-64x64.ivf"
);

const MULTI_TILE_LR_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/\
     syn-2frame-lr-switchable-768x256-8bit.ivf"
);

const FLAT_LUMA: u8 = 100;
const FLAT_CHROMA_U: u8 = 120;
const FLAT_CHROMA_V: u8 = 130;

pub(super) fn decode_context() -> DecodeContext {
    DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context")
}

fn plan_fixture(bytes: &[u8], options: &DecodeOptions) -> DecodeStreamPlan {
    decode_context().plan_bytes(bytes, *options).expect("plan")
}

pub(super) fn decode_fixture(bytes: &[u8]) -> Vec<PipelineFrame> {
    let options = DecodeOptions::default();
    decode_fixture_with_options(bytes, &options).expect("decode")
}

fn decode_fixture_with_options(
    bytes: &[u8],
    options: &DecodeOptions,
) -> Result<Vec<PipelineFrame>> {
    let context = decode_context();
    let plan = context.plan_bytes(bytes, *options).expect("plan");
    context
        .pool()
        .install(|| decode_frames_from_plan(bytes, options, &plan))
}

fn decode_frames_from_plan_on_pool(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
) -> Result<Vec<PipelineFrame>> {
    let context = decode_context();
    context
        .pool()
        .install(|| decode_frames_from_plan(bytes, options, plan))
}

fn decode_frames() -> Vec<PipelineFrame> {
    decode_fixture(TWO_FRAME_INTER_FIXTURE)
}

fn fixture_sequence_and_quantization(bytes: &[u8]) -> (SequenceHeader, QuantizationParams) {
    let (sequence, key_core) = fixture_sequence_and_key_core(bytes);
    (
        sequence,
        key_core
            .quantization_params
            .expect("key core parsed quantization params"),
    )
}

fn assert_yuv420_8bit_frames(frames: &[PipelineFrame], width: usize, height: usize) {
    let visible_size = PlaneSize::new(width, height).expect("valid visible size");
    for (index, output) in frames.iter().enumerate() {
        let PipelineDecodedFrame::Eight(frame) = output.ready_frame().expect("ready") else {
            panic!("frame {index} decoded as 10-bit");
        };
        assert_eq!(frame.bit_depth(), BitDepth::Eight, "frame {index}");
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420, "frame {index}");
        assert_eq!(frame.y().visible_size(), visible_size, "frame {index}");
    }
}

pub(super) fn frame_hashes(frames: &[PipelineFrame]) -> Vec<String> {
    frames
        .iter()
        .map(|output| {
            let PipelineDecodedFrame::Eight(frame) = output.ready_frame().expect("ready") else {
                panic!("frame decoded as 10-bit");
            };
            DecodedFrameHashInput::new(&frame).compute_hash().to_hex()
        })
        .collect()
}

fn parse_ivf_fixture<'a>(bytes: &'a [u8], name: &str) -> ParsedIvfBitstream<'a> {
    let ParsedBitstream::Ivf(parsed) = parse_bitstream_partial(bytes) else {
        panic!("{name} fixture is IVF");
    };
    assert!(parsed.error.is_none(), "{name} fixture parse error");
    assert!(parsed.warnings.is_empty(), "{name} fixture warnings");
    parsed
}

fn parse_multiref_fixture() -> ParsedIvfBitstream<'static> {
    parse_ivf_fixture(MULTIREF_FIXTURE, "multiref")
}

fn write_repacked_ivf_header(bytes: &mut Vec<u8>, header: &IvfHeader) {
    write_ivf_header(bytes, header).expect("write repacked IVF header");
}

fn write_repacked_ivf_frame(bytes: &mut Vec<u8>, pts: u64, payload: &[u8]) {
    write_ivf_frame(bytes, pts, payload).expect("write repacked IVF record");
}

fn write_original_ivf_frames(bytes: &mut Vec<u8>, frames: &[ParsedIvfFrame<'_>]) {
    for frame in frames {
        write_repacked_ivf_frame(bytes, frame.frame.pts, frame.frame.payload);
    }
}

fn decode_inter_frame_after_quantization_mutation(
    bytes: &[u8],
    mutate: impl FnOnce(&mut QuantizationParams) + Send,
) -> Result<SharedFrame<u8>> {
    decode_inter_frame_after_core_mutation(bytes, move |core| {
        mutate(
            core.quantization_params
                .as_mut()
                .expect("fixture inter core has quantization params"),
        );
    })
}

fn decode_inter_frame_after_core_mutation(
    bytes: &[u8],
    mutate: impl FnOnce(&mut FrameHeaderCore) + Send,
) -> Result<SharedFrame<u8>> {
    let context = decode_context();
    context
        .pool()
        .install(move || decode_inter_frame_after_core_mutation_inner(bytes, mutate))
}

fn decode_inter_frame_after_core_mutation_inner(
    bytes: &[u8],
    mutate: impl FnOnce(&mut FrameHeaderCore),
) -> Result<SharedFrame<u8>> {
    let options = DecodeOptions::default();
    let plan = plan_fixture(bytes, &options);
    let parsed = parse_ivf_fixture(bytes, "inter");
    let header = parsed.header.expect("fixture carries an IVF header");
    let first_ivf_frame = parsed.frames.first().expect("fixture carries a key frame");
    let [_td_envelope, sequence_envelope, key_envelope] =
        crate::pipeline::require_minimal_obu_order(first_ivf_frame.obus.as_slice())?;
    let sequence = crate::pipeline::parse_sequence(sequence_envelope)?;

    let mut candidates = plan.frame_candidates_all();
    let key_candidate = candidates.next().expect("fixture has a key candidate");
    let key_frame = crate::pipeline::decode_key_frame(
        bytes,
        &options,
        &plan,
        key_candidate,
        key_envelope,
        &sequence,
        crate::pipeline::PipelineFrameRate::from_ivf_header(header),
        None,
    )?;
    let key_core = crate::pipeline::parse_frame_core(key_envelope, &sequence)?;
    let num_ref_frames = usize::from(
        sequence
            .inter
            .as_ref()
            .expect("fixture sequence has inter config")
            .num_ref_frames,
    );
    let mut reference = crate::reference::buffer::RuntimeReferenceBuffer::new(num_ref_frames)?;
    let update = crate::pipeline::frame_ref_update_from_core(
        &key_core,
        key_envelope.offset,
        key_frame.frame_cdfs.clone(),
        key_frame.ccso_params.clone(),
        key_frame.ccso_grid.clone(),
        key_frame.motion_field.clone(),
        key_envelope.header.embedded_layer_id,
    )?;
    reference.update(0, &update);
    let frames = vec![Some(key_frame)];

    let inter_candidate = candidates.next().expect("fixture has an inter candidate");
    let mut next_unvalidated_following_ivf_record = 1;
    let prepared = crate::bitstream::byte_stream::prepare_byte_stream(bytes, &options)?;
    let crate::bitstream::byte_stream::FlatParsedBitstream::Ivf(runtime_ivf) = prepared.parsed()
    else {
        panic!("inter fixture is IVF");
    };
    let (prefix, inter_envelope) = crate::pipeline::following_inter_envelope(
        runtime_ivf,
        inter_candidate,
        &mut next_unvalidated_following_ivf_record,
    )?;
    let (store, meta) = reference.build_store_eight(&frames)?;
    let inter_state = std::sync::Arc::new(super::InterReferenceState::from_metadata(store, meta));
    let first_picture_in_tu = prefix
        .iter()
        .any(|obu| obu.header.obu_type == splot_core::types::ObuType::TemporalDelimiter);
    let mut core = super::parse_inter_frame_core(
        inter_envelope,
        &sequence,
        &inter_state,
        first_picture_in_tu,
        None,
        None,
    )?;
    mutate(&mut core);
    super::validate_inter_frame_core(&core, &sequence, inter_envelope.offset)?;
    let walk = crate::pipeline::frame_engine::walk_frame(
        &mut super::InterDecodeScratch::default(),
        &plan,
        inter_candidate,
        bytes,
        inter_envelope,
        core,
        &sequence,
        &options,
        &crate::pipeline::frame_engine::FrameSetup::Inter(&inter_state),
        BitDepth::Eight,
    )?;
    crate::pipeline::frame_engine::finish::finish_walk_inline(walk.stage)
}

pub(super) fn parse_inter_core_for_validation(
    bytes: &[u8],
) -> Result<(SequenceHeader, FrameHeaderCore, ByteOffset)> {
    let options = DecodeOptions::default();
    let plan = plan_fixture(bytes, &options);
    let parsed = parse_ivf_fixture(bytes, "inter");
    let header = parsed.header.expect("fixture carries an IVF header");
    let first_ivf_frame = parsed.frames.first().expect("fixture carries a key frame");
    let [_td_envelope, sequence_envelope, key_envelope] =
        crate::pipeline::require_minimal_obu_order(first_ivf_frame.obus.as_slice())?;
    let sequence = crate::pipeline::parse_sequence(sequence_envelope)?;

    let mut candidates = plan.frame_candidates_all();
    let key_candidate = candidates.next().expect("fixture has a key candidate");
    let key_frame = crate::pipeline::decode_key_frame(
        bytes,
        &options,
        &plan,
        key_candidate,
        key_envelope,
        &sequence,
        crate::pipeline::PipelineFrameRate::from_ivf_header(header),
        None,
    )?;
    let key_core = crate::pipeline::parse_frame_core(key_envelope, &sequence)?;
    let num_ref_frames = usize::from(
        sequence
            .inter
            .as_ref()
            .expect("fixture sequence has inter config")
            .num_ref_frames,
    );
    let mut reference = crate::reference::buffer::RuntimeReferenceBuffer::new(num_ref_frames)?;
    let update = crate::pipeline::frame_ref_update_from_core(
        &key_core,
        key_envelope.offset,
        key_frame.frame_cdfs.clone(),
        key_frame.ccso_params.clone(),
        key_frame.ccso_grid.clone(),
        key_frame.motion_field.clone(),
        key_envelope.header.embedded_layer_id,
    )?;
    reference.update(0, &update);
    let frames = vec![Some(key_frame)];

    let inter_candidate = candidates.next().expect("fixture has an inter candidate");
    let mut next_unvalidated_following_ivf_record = 1;
    let prepared = crate::bitstream::byte_stream::prepare_byte_stream(bytes, &options)?;
    let crate::bitstream::byte_stream::FlatParsedBitstream::Ivf(runtime_ivf) = prepared.parsed()
    else {
        panic!("inter fixture is IVF");
    };
    let (prefix, inter_envelope) = crate::pipeline::following_inter_envelope(
        runtime_ivf,
        inter_candidate,
        &mut next_unvalidated_following_ivf_record,
    )?;
    let (store, meta) = reference.build_store_eight(&frames)?;
    let inter_state = super::InterReferenceState::from_metadata(store, meta);
    let first_picture_in_tu = prefix
        .iter()
        .any(|obu| obu.header.obu_type == splot_core::types::ObuType::TemporalDelimiter);
    let core = super::parse_inter_frame_core(
        inter_envelope,
        &sequence,
        &inter_state,
        first_picture_in_tu,
        None,
        None,
    )?;
    Ok((sequence, core, inter_envelope.offset))
}

fn unsupported_reason(error: DecodeError) -> &'static str {
    match error {
        DecodeError::UnsupportedFeature { unsupported } => unsupported.reason(),
        _ => panic!("expected unsupported-feature error"),
    }
}

fn luma_coeff_block(quant: Vec<i32>, eob: usize, cctx_type: Option<usize>) -> LumaCoeffBlock {
    LumaCoeffBlock {
        all_zero: false,
        eob,
        quant,
        intra_ist: None,
        cctx_type,
        plane_tx_type: 0,
        use_tcq: false,
        lossless: false,
    }
}

fn all_zero_inter_coeff_block() -> LumaCoeffBlock {
    LumaCoeffBlock {
        all_zero: true,
        eob: 0,
        quant: Vec::new(),
        intra_ist: None,
        cctx_type: None,
        plane_tx_type: 0,
        use_tcq: false,
        lossless: false,
    }
}

fn read_rect_samples(
    workspace: &CurrentFrameWorkspace<u8>,
    plane: PlaneId,
    rect: PlaneRect,
) -> Vec<u8> {
    let mut samples = Vec::new();
    for row in workspace.rect_rows(plane, rect).unwrap() {
        samples.extend_from_slice(row);
    }
    samples
}

#[test]
fn inter_residual_cctx_pairs_chroma_blocks_and_applies_ddt() {
    let mut workspace = crate::pipeline::reconstruct::new_general_intra_workspace::<u8>(
        16,
        16,
        BitDepth::Eight,
        PixelFormat::Yuv420,
    )
    .unwrap();
    let rect = PlaneRect::new(0, 0, 8, 4).unwrap();
    let u_prediction = vec![128; 32];
    let v_prediction = vec![128; 32];
    workspace
        .write_rect(PlaneId::U, rect, &u_prediction, 8)
        .unwrap();
    workspace
        .write_rect(PlaneId::V, rect, &v_prediction, 8)
        .unwrap();

    let mut u_quant = vec![0; 32];
    u_quant[0] = -1;
    u_quant[1] = 1;
    let mut u_coeffs = luma_coeff_block(u_quant, 3, Some(5));
    u_coeffs.plane_tx_type = 6;
    let v_coeffs = all_zero_inter_coeff_block();
    let (want_u, want_v) = reconstruct_general_intra_chroma_cctx_pair_with_predictions(
        &u_coeffs,
        &u_prediction,
        &v_coeffs,
        &v_prediction,
        101,
        3,
        2,
        5,
        true,
        BitDepth::Eight,
    )
    .unwrap();

    let residual_blocks = vec![
        super::InterResidualBlock {
            plane: PlaneId::U,
            x: 0,
            y: 0,
            tx_size: 6,
            log2_width: 3,
            log2_height: 2,
            cctx_pair_delta: 2,
            coeffs: u_coeffs,
        },
        super::InterResidualBlock {
            plane: PlaneId::Y,
            x: 0,
            y: 0,
            tx_size: 0,
            log2_width: 2,
            log2_height: 2,
            cctx_pair_delta: 0,
            coeffs: all_zero_inter_coeff_block(),
        },
        super::InterResidualBlock {
            plane: PlaneId::V,
            x: 0,
            y: 0,
            tx_size: 6,
            log2_width: 3,
            log2_height: 2,
            cctx_pair_delta: -2,
            coeffs: v_coeffs,
        },
    ];
    let residual = super::InterResidual { block_range: 0..3 };
    let mut scratch = super::InterResidualReconScratch::default();
    super::add_inter_residual_to_workspace(
        &mut scratch,
        &mut super::mc::WorkspaceSink::Frame(&mut workspace),
        &residual,
        &residual_blocks,
        101,
        false,
        true,
        false,
        BitDepth::Eight,
        ByteOffset::new(0),
    )
    .unwrap();

    assert_eq!(read_rect_samples(&workspace, PlaneId::U, rect), want_u);
    assert_eq!(read_rect_samples(&workspace, PlaneId::V, rect), want_v);
}

#[test]
fn intrabc_residual_keeps_adst_when_inter_ddt_is_enabled() {
    let mut workspace = crate::pipeline::reconstruct::new_general_intra_workspace::<u16>(
        16,
        16,
        BitDepth::Ten,
        PixelFormat::Yuv420,
    )
    .unwrap();
    let rect = PlaneRect::new(0, 0, 16, 16).unwrap();
    workspace
        .write_rect(PlaneId::Y, rect, &[84; 256], 16)
        .unwrap();

    let mut quant = vec![0; 256];
    for (index, value) in [
        (0, 63),
        (1, -2),
        (2, -16),
        (3, 2),
        (4, -1),
        (16, -23),
        (17, -1),
        (18, 1),
        (32, -2),
        (34, 1),
        (35, 1),
        (48, -4),
        (49, 1),
        (50, 1),
        (64, 1),
        (80, 1),
        (96, 1),
    ] {
        quant[index] = value;
    }
    let mut coeffs = luma_coeff_block(quant, 22, None);
    coeffs.plane_tx_type = 1;
    let residual_blocks = vec![super::InterResidualBlock {
        plane: PlaneId::Y,
        x: 0,
        y: 0,
        tx_size: 2,
        log2_width: 4,
        log2_height: 4,
        cctx_pair_delta: 0,
        coeffs,
    }];
    let residual = super::InterResidual { block_range: 0..1 };
    let mut scratch = super::InterResidualReconScratch::default();
    super::add_inter_residual_to_workspace(
        &mut scratch,
        &mut super::mc::WorkspaceSink::Frame(&mut workspace),
        &residual,
        &residual_blocks,
        150,
        false,
        true,
        true,
        BitDepth::Ten,
        ByteOffset::new(0),
    )
    .unwrap();

    let first_row = workspace
        .rect_rows(PlaneId::Y, rect)
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(
        first_row,
        [
            81, 80, 78, 77, 75, 75, 75, 76, 78, 80, 81, 81, 79, 76, 74, 72
        ]
    );
}

#[test]
fn inter_frame_validation_admits_lossless_header_tools() {
    let context = decode_context();
    context
        .pool()
        .install(|| -> Result<()> {
            let (mut sequence, mut core, offset) =
                parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE)?;
            sequence
                .inter
                .as_mut()
                .expect("fixture sequence has inter config")
                .enable_tip = true;
            core.inter
                .as_mut()
                .expect("fixture inter core has control region")
                .tip_frame_mode = Some(TipFrameMode::AsRef);

            let quant = core
                .quantization_params
                .as_mut()
                .expect("fixture inter core has quantization params");
            quant.base_q_idx = 0;
            let lossless = core
                .lossless_info
                .as_mut()
                .expect("fixture inter core has lossless facts");
            lossless.lossless_array.fill(true);
            lossless.coded_lossless = true;
            lossless.has_lossless_segment = true;
            lossless.allow_tcq = false;
            lossless.allow_parity_hiding = false;

            super::validate_inter_frame_core(&core, &sequence, offset)
        })
        .expect("lossless inter headers should reach block-specific gates");
}

#[test]
fn inter_frame_validation_admits_active_gdf_header_tool() {
    let context = decode_context();
    context
        .pool()
        .install(|| -> Result<()> {
            let (sequence, mut core, offset) =
                parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE)?;
            let gdf = core
                .gdf_params
                .as_mut()
                .expect("fixture inter core has GDF params");
            gdf.gdf_frame_enable = true;
            gdf.gdf_per_block = Some(false);
            gdf.gdf_pic_qc_idx = Some(0);
            gdf.gdf_pic_scale_idx = Some(0);

            super::validate_inter_frame_core(&core, &sequence, offset)
        })
        .expect("active inter GDF headers should reach block-specific gates");
}

#[test]
fn inter_frame_validation_admits_delta_q_and_rejects_missing_params() {
    let context = decode_context();
    context
        .pool()
        .install(|| -> Result<()> {
            let (sequence, mut core, offset) =
                parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE)?;
            let delta_q = core
                .delta_q_params
                .as_mut()
                .expect("fixture inter core has delta-Q params");
            delta_q.delta_q_present = true;
            delta_q.delta_q_res = 2;

            super::validate_inter_frame_core(&core, &sequence, offset)
                .expect("active delta-Q is handled by the inter block walk");

            core.delta_q_params = None;
            let error = super::validate_inter_frame_core(&core, &sequence, offset)
                .expect_err("missing delta-Q params must stay fail-closed");
            assert_eq!(unsupported_reason(error), "inter_unsupported_frame_tools");
            Ok(())
        })
        .unwrap();
}

#[test]
fn two_frame_inter_fixture_decodes_both_frames_bit_exact() {
    let frames = decode_frames();
    assert_eq!(
        frames.len(),
        2,
        "the stream decodes a key frame + one inter frame"
    );
    assert_yuv420_8bit_frames(&frames, 64, 64);

    for (index, output) in frames.iter().enumerate() {
        let frame = output.frame();
        assert!(
            frame.y().samples().iter().all(|&s| s == FLAT_LUMA),
            "frame {index} luma must be flat {FLAT_LUMA}"
        );
        assert!(
            frame
                .u()
                .unwrap()
                .samples()
                .iter()
                .all(|&s| s == FLAT_CHROMA_U),
            "frame {index} U must be flat {FLAT_CHROMA_U}"
        );
        assert!(
            frame
                .v()
                .unwrap()
                .samples()
                .iter()
                .all(|&s| s == FLAT_CHROMA_V),
            "frame {index} V must be flat {FLAT_CHROMA_V}"
        );
    }
}

#[test]
fn multi_tile_inter_fixture_enforces_tile_count_limit() {
    let options = DecodeOptions::default()
        .with_limits(DecodeLimits::default().with_max_tile_count(DecodeLimitThreshold::Max(1)));
    let Err(error) = decode_fixture_with_options(MULTI_TILE_LR_FIXTURE, &options) else {
        panic!("two tile columns must exceed a one-tile resource limit");
    };
    let DecodeError::Limit { source } = error else {
        panic!("expected tile-count limit error");
    };
    assert_eq!(source.name(), DecodeLimitName::MaxTileCount);
    assert_eq!(source.actual(), Some(2));
}

#[test]
fn ten_bit_flex_mvres_inter_fixture_decodes_avm_bit_exact() {
    let frames = decode_fixture(TWO_FRAME_INTER_10BIT_FIXTURE);
    assert_eq!(
        frames.len(),
        2,
        "the 10-bit flex-mvres stream decodes a key frame + one inter frame"
    );
    assert_eq!(
        ten_bit_frame_hashes(&frames),
        [
            "973eb3fc4b112c865f939dc1339824ca0b2a1522ca2b5ec70311afb459436e2d",
            "071c44ed4bf3bce19d530834f741bc852b9eff5163c1f3012ea94ad1f5a890c5"
        ],
        "frame hashes pinned from the avmdec --i420 --rawvideo byte-identical output"
    );
}

fn ten_bit_frame_hashes(frames: &[PipelineFrame]) -> Vec<String> {
    frames
        .iter()
        .map(|output| {
            let PipelineDecodedFrame::Ten(frame) = output.ready_frame().expect("ready") else {
                panic!("frame decoded as 8-bit");
            };
            DecodedFrameHashInput::new(&frame).compute_hash().to_hex()
        })
        .collect()
}

#[test]
fn deblock_active_inter_fixture_decodes_avm_bit_exact() {
    let frames = decode_fixture(DEBLOCK_INTER_10BIT_FIXTURE);
    assert_eq!(
        frames.len(),
        2,
        "key frame + one deblock-active inter frame"
    );
    assert_eq!(
        ten_bit_frame_hashes(&frames),
        [
            "9978e070c5ec6d67a4338ce86cfac42b4a5e833f9502a91da6bd3f3c3220239c",
            "6232ef9d0a3a82875a7d48341029b3cbc0ba03fe452f758851203bd00004208b"
        ],
        "frame hashes pinned from the avmdec --i420 --rawvideo byte-identical output"
    );
}

#[test]
fn cdef_active_inter_fixture_decodes_avm_bit_exact() {
    let frames = decode_fixture(CDEF_INTER_10BIT_FIXTURE);
    assert_eq!(frames.len(), 2, "key frame + one CDEF-active inter frame");
    assert_eq!(
        ten_bit_frame_hashes(&frames),
        [
            "a3b6f98ab490ab31d2febd7d238d543ff3826f2ff6ef53c167d17bfb66bfb254",
            "55f282e51d2df475fa845926b4328dbf0061e811ec283417be375a6d43860ec3"
        ],
        "frame hashes pinned from the avmdec --i420 --rawvideo byte-identical output"
    );
}

#[test]
fn ccso_active_inter_fixture_decodes_avm_bit_exact() {
    let frames = decode_fixture(CCSO_INTER_10BIT_FIXTURE);
    assert_eq!(frames.len(), 2, "key frame + one CCSO-active inter frame");
    assert_eq!(
        ten_bit_frame_hashes(&frames),
        [
            "95399be9043a0fd3fb501d4708303825d82c189ba5e0b8eed4f41f3f005b3137",
            "0ffefc28c772da1b372dafdeff9bce414449a20f36e170ec24fffbefd5780cd3"
        ],
        "frame hashes pinned from the avmdec --i420 --rawvideo byte-identical output"
    );
}

#[test]
fn comp_ref_allowed_follows_the_spec_block_size_arms() {
    for (n4w, n4h, allowed) in [
        (1, 1, false),
        (1, 2, false),
        (2, 1, false),
        (1, 4, true),
        (4, 1, true),
        (2, 2, true),
        (16, 16, true),
    ] {
        assert_eq!(
            super::block::is_comp_ref_allowed(n4w, n4h),
            allowed,
            "is_comp_ref_allowed({n4w}, {n4h})"
        );
    }
}

#[test]
fn simple_path_interintra_fixture_decodes_avm_bit_exact() {
    let frames = decode_fixture(SIMPLE_INTERINTRA_10BIT_FIXTURE);
    assert_eq!(frames.len(), 3, "key frame + two inter frames");
    assert_eq!(
        ten_bit_frame_hashes(&frames),
        [
            "73df53de5404c338dd1318408350d0975316c28927bcdf7036d5efd442eb2d51",
            "2981a14f81c61f8cca02eceefd096fba83c406b5196224007cf8681620d9bab3",
            "8f0c833c194e738ab2b07654bc75fb38189bfc4b1e13e90ff6d266fcaf45e110"
        ],
        "frame hashes pinned from the avmdec --i420 --rawvideo byte-identical output"
    );
}

#[test]
fn fractional_quarter_pel_intrabc_fixture_decodes_avm_bit_exact() {
    let frames = decode_fixture(FRACTIONAL_INTRABC_FIXTURE);
    assert_eq!(frames.len(), 1, "one closed-loop key frame");
    assert_eq!(
        frame_hashes(&frames),
        ["5e6a9eac61011e29f965e53a7fb8f2e8278bae53c1370772ea96344cf8e56dea"],
        "AVM-pinned output proves the quarter-pel IntrABC BILINEAR path"
    );
}

#[test]
fn same_ref_compound_fixture_decodes_avm_bit_exact() {
    let frames = decode_fixture(SAMEREF_COMPOUND_10BIT_FIXTURE);
    assert_eq!(frames.len(), 2, "key frame + one same-ref compound frame");
    assert_eq!(
        ten_bit_frame_hashes(&frames),
        [
            "0f86a97c44d252fe35ffff48a3604ab7a8fb6af6de9b53309b917c1192b39311",
            "9aed01f7117b8ca011ee4a3a5bd128c8ba23c00a575a344fd53cb6d324093b42"
        ],
        "frame hashes pinned from the avmdec --i420 --rawvideo byte-identical output"
    );
}

#[test]
fn ccso_reference_reuse_inter_fixture_decodes_avm_bit_exact() {
    let frames = decode_fixture(CCSO_REUSE_INTER_10BIT_FIXTURE);
    assert_eq!(frames.len(), 2, "key frame + one CCSO reuse inter frame");
    assert_eq!(
        ten_bit_frame_hashes(&frames),
        [
            "835ba046166243042ec013a41600f81468511237a3122a0bddaa9536f9b697da",
            "092e512e99679ab8238e4aad63635146d339bc63c9a59a17a26b2afb924a6dfc"
        ],
        "frame hashes pinned from the avmdec --i420 --rawvideo byte-identical output"
    );
}

#[test]
fn partitioned_intra_prediction_inter_fixture_decodes_avm_bit_exact() {
    let frames = decode_fixture(TXSPLIT_INTRA_INTER_10BIT_FIXTURE);
    assert_eq!(
        frames.len(),
        2,
        "key frame + one inter frame with a perpendicular multi-unit intra split"
    );
    assert_eq!(
        ten_bit_frame_hashes(&frames),
        [
            "49dcc6ac122a807aa0412154b398485b5afb8b745af91871a69c66378700fae5",
            "708439b34b7954f9196fe7d26f87770d29492f49797ed65ffcb74bf937911856"
        ],
        "frame hashes pinned from the avmdec --i420 --rawvideo byte-identical output"
    );
}

#[test]
fn inter_frame_is_a_bit_exact_copy_of_the_key_frame() {
    let frames = decode_frames();
    let key = frames[0].frame();
    let inter = frames[1].frame();
    assert_eq!(key.y().samples(), inter.y().samples(), "luma copy");
    assert_eq!(
        key.u().unwrap().samples(),
        inter.u().unwrap().samples(),
        "U copy"
    );
    assert_eq!(
        key.v().unwrap().samples(),
        inter.v().unwrap().samples(),
        "V copy"
    );
}

#[test]
fn subpel_fixture_decodes_two_frames() {
    let frames = decode_fixture(TWO_FRAME_SUBPEL_FIXTURE);
    assert_eq!(
        frames.len(),
        2,
        "the sub-pel stream decodes a key frame + one inter frame"
    );
    assert_yuv420_8bit_frames(&frames, 64, 64);
}

#[test]
fn subpel_inter_frame_differs_from_key_frame() {
    let frames = decode_fixture(TWO_FRAME_SUBPEL_FIXTURE);
    let key = frames[0].frame();
    let inter = frames[1].frame();
    assert_ne!(
        key.y().samples(),
        inter.y().samples(),
        "the sub-pel inter luma must differ from the key luma (real fractional MC)"
    );
}

#[test]
fn subpel_fixture_per_frame_hash_is_stable() {
    let frames = decode_fixture(TWO_FRAME_SUBPEL_FIXTURE);
    let hashes = frame_hashes(&frames);
    assert_eq!(
        hashes[0], "8a6751d4517073bad0bbe71f4b5537df8e8b0bfee85fcd6af1ac2d5878dd59e8",
        "sub-pel key-frame hash"
    );
    assert_eq!(
        hashes[1], "4c2443d95b38cee9a574ba1166a1fe15d6f2b5d20de070001d31db15a661896e",
        "sub-pel inter-frame hash"
    );
    assert_ne!(hashes[0], hashes[1], "the sub-pel frames must differ");
}

#[test]
fn two_frame_inter_fixture_per_frame_hash_is_stable() {
    let frames = decode_frames();
    let hashes = frame_hashes(&frames);
    assert_eq!(hashes[0], hashes[1], "inter frame hash == key frame hash");
}

#[test]
fn residual_fixture_decodes_two_frames() {
    let frames = decode_fixture(TWO_FRAME_RESIDUAL_FIXTURE);
    assert_eq!(
        frames.len(),
        2,
        "the residual stream decodes a key frame + one inter frame"
    );
    assert_yuv420_8bit_frames(&frames, 64, 64);
}

#[test]
fn residual_reconstruction_uses_nonzero_luma_dc_quantizer_delta() {
    let baseline =
        decode_inter_frame_after_quantization_mutation(TWO_FRAME_RESIDUAL_FIXTURE, |_| {})
            .expect("baseline residual frame decodes");
    let nonzero =
        decode_inter_frame_after_quantization_mutation(TWO_FRAME_RESIDUAL_FIXTURE, |quant| {
            quant.delta_q_y_dc = 1;
        })
        .expect("non-zero DeltaQYDc residual frame decodes");
    assert_ne!(
        baseline.y().samples(),
        nonzero.y().samples(),
        "DeltaQYDc must change reconstructed luma residuals"
    );
}

#[test]
fn skip_one_inter_allows_nonzero_effective_quantizer_deltas() {
    decode_inter_frame_after_quantization_mutation(TWO_FRAME_INTER_FIXTURE, |quant| {
        quant.delta_q_y_dc = 1;
    })
    .expect("skip == 1 reads no residual and accepts the frame quantizer state");
}

#[test]
fn residual_inter_frame_differs_from_key_frame() {
    let frames = decode_fixture(TWO_FRAME_RESIDUAL_FIXTURE);
    let key = frames[0].frame();
    let inter = frames[1].frame();
    assert!(
        key.y().samples().iter().all(|&s| s == FLAT_LUMA),
        "key luma must be flat {FLAT_LUMA}"
    );
    assert_ne!(
        key.y().samples(),
        inter.y().samples(),
        "the residual inter luma must differ from the flat key luma (real residual)"
    );
    assert_eq!(
        key.u().unwrap().samples(),
        inter.u().unwrap().samples(),
        "U: no chroma residual"
    );
    assert_eq!(
        key.v().unwrap().samples(),
        inter.v().unwrap().samples(),
        "V: no chroma residual"
    );
    assert!(
        inter
            .u()
            .unwrap()
            .samples()
            .iter()
            .all(|&s| s == FLAT_CHROMA_U),
        "inter U flat {FLAT_CHROMA_U}"
    );
    assert!(
        inter
            .v()
            .unwrap()
            .samples()
            .iter()
            .all(|&s| s == FLAT_CHROMA_V),
        "inter V flat {FLAT_CHROMA_V}"
    );
}

#[test]
fn residual_fixture_per_frame_hash_is_stable() {
    let frames = decode_fixture(TWO_FRAME_RESIDUAL_FIXTURE);
    let hashes = frame_hashes(&frames);
    assert_eq!(
        hashes[0], "ce9c46b1078b9dd593254837ead7dcd6cee8b3ec6cc3c7d34f54fb08df703979",
        "residual key-frame hash"
    );
    assert_eq!(
        hashes[1], "6bc96c12710ebe225b994c8e70e253e7159cd3fe49da61de5ad2558c207e26d8",
        "residual inter-frame hash"
    );
    assert_ne!(
        hashes[0], hashes[1],
        "the residual inter frame must differ from the key frame"
    );
}

#[test]
fn nonzero_y_dc_quantizer_delta_fixture_is_bit_exact() {
    let (_, core, _) = decode_context()
        .pool()
        .install(|| parse_inter_core_for_validation(TWO_FRAME_Y_DC_DELTA_FIXTURE))
        .expect("quantizer-delta fixture inter header parses");
    let quantization = core
        .quantization_params
        .expect("quantizer-delta fixture has frame quantization params");
    assert_eq!(
        (
            quantization.delta_q_y_dc,
            quantization.delta_q_u_dc,
            quantization.delta_q_u_ac,
            quantization.delta_q_v_dc,
            quantization.delta_q_v_ac,
        ),
        (1, 0, 0, 0, 0)
    );

    let frames = decode_fixture(TWO_FRAME_Y_DC_DELTA_FIXTURE);
    assert_eq!(
        frame_hashes(&frames),
        [
            "ebf2ba02fa61281e66533bc142260d49971a96101442d7df7d099b1d2be3bad5",
            "e73a3b0168597953992650452b153d6d316f649254b2493864fb6d320a3d8f53",
        ]
    );
    let key = frames[0].frame();
    let inter = frames[1].frame();
    assert_ne!(key.y().samples(), inter.y().samples());
    assert_eq!(key.u().unwrap().samples(), inter.u().unwrap().samples());
    assert_eq!(key.v().unwrap().samples(), inter.v().unwrap().samples());
}

#[test]
fn mvstack_fixture_decodes_two_frames() {
    let frames = decode_fixture(TWO_FRAME_MVSTACK_FIXTURE);
    assert_eq!(
        frames.len(),
        2,
        "the multi-block stream decodes a key frame + one inter frame"
    );
    assert_yuv420_8bit_frames(&frames, 64, 64);
}

#[test]
fn mvstack_fixture_per_frame_hash_is_stable() {
    let frames = decode_fixture(TWO_FRAME_MVSTACK_FIXTURE);
    let hashes = frame_hashes(&frames);
    assert_eq!(
        hashes[0], "37d5a851609575dcceec47aa4b53043fa04f36cb483c40925913b8adfd91504f",
        "multi-block key-frame hash"
    );
    assert_eq!(
        hashes[1], "b39afe593c1046b080efea9c8bf76242dba2a4965a556d7ed31bcf0fca444fc1",
        "multi-block inter-frame hash"
    );
    assert_ne!(
        hashes[0], hashes[1],
        "the multi-block inter frame must differ from the key frame"
    );
}

#[test]
fn multi_sb_fixture_decodes_two_frames() {
    let frames = decode_fixture(MULTI_SB_INTER_FIXTURE);
    assert_eq!(
        frames.len(),
        2,
        "the multi-superblock stream decodes a key frame + one inter frame"
    );
    assert_yuv420_8bit_frames(&frames, 128, 64);
}

#[test]
fn multi_sb_fixture_per_frame_hash_is_stable() {
    let frames = decode_fixture(MULTI_SB_INTER_FIXTURE);
    let hashes = frame_hashes(&frames);
    assert_eq!(
        hashes[0], "2dc3b82d7f75dd5f400474fbf370a9acc2e631f65e2cc1263d0ec0684b14da15",
        "multi-superblock key-frame hash"
    );
    assert_eq!(
        hashes[1], "dc9b4c4aef4e6dc1afa43ed16a93c17dd2fab9c1e61b5ab97dbae863d62a7ebd",
        "multi-superblock inter-frame hash"
    );
    assert_ne!(
        hashes[0], hashes[1],
        "the multi-superblock inter frame must differ from the key frame (real cross-SB MV shift)"
    );
}

#[test]
fn multi_tile_inter_fixture_decodes_bit_exact() {
    for threads in [1usize, 4] {
        let options = DecodeOptions::default();
        let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(threads)))
            .expect("context");
        let plan = context
            .plan_bytes(MULTI_TILE_INTER_FIXTURE, options)
            .expect("plan");
        let frames = context
            .pool()
            .install(|| decode_frames_from_plan(MULTI_TILE_INTER_FIXTURE, &options, &plan))
            .expect("decode");
        assert_yuv420_8bit_frames(&frames, 128, 64);
        assert_eq!(
            frame_hashes(&frames),
            [
                "2dc3b82d7f75dd5f400474fbf370a9acc2e631f65e2cc1263d0ec0684b14da15",
                "dc9b4c4aef4e6dc1afa43ed16a93c17dd2fab9c1e61b5ab97dbae863d62a7ebd"
            ],
            "two-tile output must match the pinned avmdec frames"
        );
    }
}

#[test]
fn multi_tile_lr_fixture_decodes_bit_exact() {
    for threads in [1usize, 4] {
        let options = DecodeOptions::default();
        let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(threads)))
            .expect("context");
        let plan = context
            .plan_bytes(MULTI_TILE_LR_FIXTURE, options)
            .expect("plan");
        let frames = context
            .pool()
            .install(|| decode_frames_from_plan(MULTI_TILE_LR_FIXTURE, &options, &plan))
            .expect("decode");
        assert_yuv420_8bit_frames(&frames, 768, 256);
        assert_eq!(
            frame_hashes(&frames),
            [
                "40567c0d82f8c0c50e4ce59fd4630ec6dd1049e4405321992e7c40f9047630b2",
                "5bdc64e0d79ebbfea730882ad0c6f678307c764d91e20fa1902fb8cc8738bffe"
            ],
            "multi-tile LR output must match the pinned avmdec frames"
        );
    }
}

#[test]
fn grid_fixture_decodes_avm_bit_exact() {
    let frames = decode_fixture(GRID_INTER_FIXTURE);
    assert_eq!(frames.len(), 2, "key frame + one 2-D-grid inter frame");
    let hashes = frame_hashes(&frames);
    assert_eq!(
        hashes,
        [
            "5619e639914803867ca0bdeb12bff97e808788607f992c661a7bcfc0bea4911a",
            "f23ded7e9197d7c9b0a2fdc5cdc649c079cd1fb8a1c79e913b72fb74f0c502db"
        ],
        "frame hashes pinned from the avmdec --i420 --rawvideo byte-identical output"
    );
}

#[test]
fn mvorder_fixture_decodes_two_frames() {
    let frames = decode_fixture(MVORDER_INTER_FIXTURE);
    assert_eq!(
        frames.len(),
        2,
        "the distinct-MV stream decodes a key frame + one inter frame"
    );
    assert_yuv420_8bit_frames(&frames, 64, 64);
}

#[test]
fn mvorder_fixture_per_frame_hash_is_stable() {
    let frames = decode_fixture(MVORDER_INTER_FIXTURE);
    let hashes = frame_hashes(&frames);
    assert_eq!(
        hashes[0], "3ddad4a90c482c106f9389ef55bc87beeaf772f4bec2041da4555bbd8deb6142",
        "distinct-MV key-frame hash"
    );
    assert_eq!(
        hashes[1], "3c2a8c85c4ba4be4fa82aecbefe92baa1567f2a9c45ea88f8275c21414480ad9",
        "distinct-MV inter-frame hash"
    );
    assert_ne!(
        hashes[0], hashes[1],
        "the distinct-MV inter frame must differ from the key frame"
    );
}

const MULTIREF_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-3frame-multiref-64x64.ivf");

fn repack_multiref_first_inter_into_leading_ivf_record() -> Vec<u8> {
    let parsed = parse_multiref_fixture();
    assert!(!parsed.frames.is_empty());
    assert_eq!(parsed.frames.len(), 3);
    assert_eq!(parsed.frames[1].obus.len(), 2);

    let first_inter_td_end = obu_end_in_ivf_payload(&parsed.frames[1], 0);
    let first_inter_payload = parsed.frames[1].frame.payload;

    let mut leading_payload = Vec::new();
    leading_payload.extend_from_slice(parsed.frames[0].frame.payload);
    leading_payload.extend_from_slice(&first_inter_payload[first_inter_td_end..]);

    let mut header = parsed.header.expect("source fixture has an IVF header");
    header.frame_count = 2;

    let mut bytes = Vec::new();
    write_repacked_ivf_header(&mut bytes, &header);
    write_repacked_ivf_frame(&mut bytes, parsed.frames[0].frame.pts, &leading_payload);
    write_repacked_ivf_frame(
        &mut bytes,
        parsed.frames[2].frame.pts,
        parsed.frames[2].frame.payload,
    );

    bytes
}

fn repack_multiref_last_two_frames_into_one_ivf_record() -> Vec<u8> {
    let parsed = parse_multiref_fixture();
    assert_eq!(parsed.frames.len(), 3);

    let mut header = parsed.header.expect("multiref fixture has an IVF header");
    header.frame_count = 2;

    let mut bytes = Vec::new();
    write_repacked_ivf_header(&mut bytes, &header);
    write_repacked_ivf_frame(
        &mut bytes,
        parsed.frames[0].frame.pts,
        parsed.frames[0].frame.payload,
    );

    let mut grouped_inter_payload = Vec::new();
    grouped_inter_payload.extend_from_slice(parsed.frames[1].frame.payload);
    grouped_inter_payload.extend_from_slice(parsed.frames[2].frame.payload);
    write_repacked_ivf_frame(
        &mut bytes,
        parsed.frames[1].frame.pts,
        &grouped_inter_payload,
    );

    bytes
}

fn obu_end_in_ivf_payload(frame: &ParsedIvfFrame<'_>, obu_index: usize) -> usize {
    let envelope = frame.obus[obu_index];
    let frame_start = frame.frame.payload_offset.get();
    let end = envelope.offset.get() + u64::from(envelope.size);
    usize::try_from(
        end.checked_sub(frame_start)
            .expect("OBU belongs to frame payload"),
    )
    .expect("OBU payload-relative end fits usize")
}

fn repack_multiref_first_inter_td_separate_from_tile_group() -> Vec<u8> {
    let parsed = parse_multiref_fixture();
    assert_eq!(parsed.frames.len(), 3);
    assert_eq!(parsed.frames[1].obus.len(), 2);

    let mut header = parsed.header.expect("multiref fixture has an IVF header");
    header.frame_count = 3;

    let first_inter_td_end = obu_end_in_ivf_payload(&parsed.frames[1], 0);
    let first_inter_payload = parsed.frames[1].frame.payload;

    let mut bytes = Vec::new();
    write_repacked_ivf_header(&mut bytes, &header);
    write_repacked_ivf_frame(
        &mut bytes,
        parsed.frames[0].frame.pts,
        parsed.frames[0].frame.payload,
    );
    write_repacked_ivf_frame(
        &mut bytes,
        parsed.frames[1].frame.pts,
        &first_inter_payload[..first_inter_td_end],
    );

    let mut record_leading_tile_group_payload = Vec::new();
    record_leading_tile_group_payload.extend_from_slice(&first_inter_payload[first_inter_td_end..]);
    record_leading_tile_group_payload.extend_from_slice(parsed.frames[2].frame.payload);
    write_repacked_ivf_frame(
        &mut bytes,
        parsed.frames[2].frame.pts,
        &record_leading_tile_group_payload,
    );

    bytes
}

fn repack_multiref_first_inter_with_extra_sequence_header() -> Vec<u8> {
    let parsed = parse_multiref_fixture();
    assert_eq!(parsed.frames.len(), 3);
    assert_eq!(parsed.frames[0].obus.len(), 3);

    let mut header = parsed.header.expect("multiref fixture has an IVF header");
    header.frame_count = 3;

    let sequence_start = obu_end_in_ivf_payload(&parsed.frames[0], 0);
    let sequence_end = obu_end_in_ivf_payload(&parsed.frames[0], 1);
    let sequence_obu = &parsed.frames[0].frame.payload[sequence_start..sequence_end];

    let mut state_prefixed_inter_payload = Vec::new();
    state_prefixed_inter_payload.extend_from_slice(sequence_obu);
    state_prefixed_inter_payload.extend_from_slice(parsed.frames[1].frame.payload);

    let mut bytes = Vec::new();
    write_repacked_ivf_header(&mut bytes, &header);
    write_repacked_ivf_frame(
        &mut bytes,
        parsed.frames[0].frame.pts,
        parsed.frames[0].frame.payload,
    );
    write_repacked_ivf_frame(
        &mut bytes,
        parsed.frames[1].frame.pts,
        &state_prefixed_inter_payload,
    );
    write_repacked_ivf_frame(
        &mut bytes,
        parsed.frames[2].frame.pts,
        parsed.frames[2].frame.payload,
    );

    bytes
}

fn repack_multiref_first_inter_with_trailing_sequence_header() -> Vec<u8> {
    let parsed = parse_multiref_fixture();
    assert_eq!(parsed.frames.len(), 3);
    assert_eq!(parsed.frames[0].obus.len(), 3);

    let mut header = parsed.header.expect("multiref fixture has an IVF header");
    header.frame_count = 3;

    let sequence_start = obu_end_in_ivf_payload(&parsed.frames[0], 0);
    let sequence_end = obu_end_in_ivf_payload(&parsed.frames[0], 1);
    let sequence_obu = &parsed.frames[0].frame.payload[sequence_start..sequence_end];

    let mut state_trailing_inter_payload = Vec::new();
    state_trailing_inter_payload.extend_from_slice(parsed.frames[1].frame.payload);
    state_trailing_inter_payload.extend_from_slice(sequence_obu);

    let mut bytes = Vec::new();
    write_repacked_ivf_header(&mut bytes, &header);
    write_repacked_ivf_frame(
        &mut bytes,
        parsed.frames[0].frame.pts,
        parsed.frames[0].frame.payload,
    );
    write_repacked_ivf_frame(
        &mut bytes,
        parsed.frames[1].frame.pts,
        &state_trailing_inter_payload,
    );
    write_repacked_ivf_frame(
        &mut bytes,
        parsed.frames[2].frame.pts,
        parsed.frames[2].frame.payload,
    );

    bytes
}

fn append_multiref_third_frame_as_fourth_ivf_record() -> Vec<u8> {
    let parsed = parse_multiref_fixture();
    assert_eq!(parsed.frames.len(), 3);

    let mut header = parsed.header.expect("multiref fixture has an IVF header");
    header.frame_count = 4;

    let mut bytes = Vec::new();
    write_repacked_ivf_header(&mut bytes, &header);
    write_original_ivf_frames(&mut bytes, &parsed.frames);
    write_repacked_ivf_frame(&mut bytes, 3, parsed.frames[2].frame.payload);

    bytes
}

fn append_future_state_record_after_fourth_multiref_candidate() -> Vec<u8> {
    let parsed = parse_multiref_fixture();
    assert_eq!(parsed.frames.len(), 3);
    assert_eq!(parsed.frames[0].obus.len(), 3);

    let mut header = parsed.header.expect("multiref fixture has an IVF header");
    header.frame_count = 5;

    let sequence_start = obu_end_in_ivf_payload(&parsed.frames[0], 0);
    let sequence_end = obu_end_in_ivf_payload(&parsed.frames[0], 1);
    let sequence_obu = &parsed.frames[0].frame.payload[sequence_start..sequence_end];

    let mut bytes = Vec::new();
    write_repacked_ivf_header(&mut bytes, &header);
    write_original_ivf_frames(&mut bytes, &parsed.frames);
    write_repacked_ivf_frame(&mut bytes, 3, parsed.frames[2].frame.payload);
    write_repacked_ivf_frame(&mut bytes, 4, sequence_obu);

    bytes
}

const COMPOUND_AVERAGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-3frame-compound-average-64x64.ivf"
);

const OPFL_REFINE_ALL_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-8frame-opfl-refine-all-64x64-q120.ivf"
);

#[test]
fn multiref_fixture_decodes_three_frames_bit_exact() {
    let frames = decode_fixture(MULTIREF_FIXTURE);
    assert_eq!(
        frames.len(),
        3,
        "the stream decodes a key frame + two inter frames"
    );
    assert_yuv420_8bit_frames(&frames, 64, 64);

    let expected: [(u8, u8, u8); 3] = [(100, 120, 130), (160, 90, 70), (160, 90, 70)];
    for (index, output) in frames.iter().enumerate() {
        let frame = output.frame();
        let (y, u, v) = expected[index];
        assert!(
            frame.y().samples().iter().all(|&s| s == y),
            "frame {index} luma must be flat {y}"
        );
        assert!(
            frame.u().unwrap().samples().iter().all(|&s| s == u),
            "frame {index} U must be flat {u}"
        );
        assert!(
            frame.v().unwrap().samples().iter().all(|&s| s == v),
            "frame {index} V must be flat {v}"
        );
    }
}

#[test]
fn multiref_fixture_decodes_when_two_frame_units_share_one_ivf_record() {
    let repacked = repack_multiref_last_two_frames_into_one_ivf_record();
    let original = decode_fixture(MULTIREF_FIXTURE);
    let grouped = decode_fixture(&repacked);

    assert_eq!(grouped.len(), original.len());
    for (index, (actual, expected)) in grouped.iter().zip(original.iter()).enumerate() {
        assert_eq!(
            actual.frame().y().samples(),
            expected.frame().y().samples(),
            "repacked frame {index} luma"
        );
        assert_eq!(
            actual.frame().u().unwrap().samples(),
            expected.frame().u().unwrap().samples(),
            "repacked frame {index} U"
        );
        assert_eq!(
            actual.frame().v().unwrap().samples(),
            expected.frame().v().unwrap().samples(),
            "repacked frame {index} V"
        );
    }
}

#[test]
fn multiref_fixture_rejects_when_inter_tile_group_starts_ivf_record() {
    let repacked = repack_multiref_first_inter_td_separate_from_tile_group();
    let options = DecodeOptions::default();
    let plan = plan_fixture(&repacked, &options);
    let Err(error) = decode_frames_from_plan_on_pool(&repacked, &options, &plan) else {
        panic!("record-leading tile group must fail closed");
    };
    assert_eq!(unsupported_reason(error), "unexpected_inter_obu_order");
}

#[test]
fn wienerns_header_status_reports_precise_tile_frontier() {
    let error = crate::pipeline::incomplete_intra_header_error(
        FrameHeaderParseStatus::StoppedBeforeWienerNsFilter {
            feature_id: "AV2-5.18.7-SEGMENTATION-TILING",
        },
        ByteOffset::new(74),
    );
    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("Wiener NS frontier must be an unsupported-feature error");
    };

    assert_eq!(unsupported.reason(), "unsupported_wienerns_filter");
    assert_eq!(unsupported.spec_section(), "5.18.7.11");
    assert_eq!(unsupported.byte_offset(), Some(ByteOffset::new(74)));
    assert!(
        unsupported.message().contains("read_wienerns_filter"),
        "message should name the exact unmodeled parser subroutine"
    );
}

#[test]
fn non_wienerns_header_status_keeps_generic_incomplete_frontier() {
    let error = crate::pipeline::incomplete_intra_header_error(
        FrameHeaderParseStatus::ActivationFieldsOnly,
        ByteOffset::new(74),
    );
    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("incomplete header fallback must be an unsupported-feature error");
    };

    assert_eq!(unsupported.reason(), "incomplete_frame_header");
    assert_eq!(unsupported.spec_section(), "7.1");
    assert_eq!(unsupported.byte_offset(), Some(ByteOffset::new(74)));
}

#[test]
fn multiref_runtime_decodes_first_inter_from_leading_ivf_record() {
    let repacked = repack_multiref_first_inter_into_leading_ivf_record();
    let original = decode_fixture(MULTIREF_FIXTURE);
    let grouped = decode_fixture(&repacked);

    assert_eq!(grouped.len(), original.len());
    for (index, (actual, expected)) in grouped.iter().zip(original.iter()).enumerate() {
        assert_eq!(
            actual.frame().y().samples(),
            expected.frame().y().samples(),
            "repacked frame {index} luma"
        );
        assert_eq!(
            actual.frame().u().unwrap().samples(),
            expected.frame().u().unwrap().samples(),
            "repacked frame {index} U"
        );
        assert_eq!(
            actual.frame().v().unwrap().samples(),
            expected.frame().v().unwrap().samples(),
            "repacked frame {index} V"
        );
    }
}

#[test]
fn multiref_runtime_rejects_state_obu_before_following_inter_candidate() {
    let repacked = repack_multiref_first_inter_with_extra_sequence_header();
    let options = DecodeOptions::default();
    let plan = plan_fixture(&repacked, &options);
    assert_eq!(
        plan.frame_candidate_count(),
        3,
        "test fixture must retain the key plus two inter frame candidates"
    );
    let Err(error) = decode_frames_from_plan_on_pool(&repacked, &options, &plan) else {
        panic!("extra state before a following inter candidate must fail closed");
    };
    assert_eq!(unsupported_reason(error), "unexpected_inter_obu_order");
}

#[test]
fn multiref_runtime_rejects_state_obu_after_inter_candidate_before_next_frame() {
    let repacked = repack_multiref_first_inter_with_trailing_sequence_header();
    let options = DecodeOptions::default();
    let plan = plan_fixture(&repacked, &options);
    assert_eq!(
        plan.frame_candidate_count(),
        3,
        "test fixture must retain the key plus two inter frame candidates"
    );
    let Err(error) = decode_frames_from_plan_on_pool(&repacked, &options, &plan) else {
        panic!("state after one inter candidate and before the next must fail closed");
    };
    assert_eq!(unsupported_reason(error), "unexpected_inter_obu_order");
}

#[test]
fn four_frame_multiref_decodes_without_total_refs_gate() {
    let four_frame = append_multiref_third_frame_as_fourth_ivf_record();
    let options = DecodeOptions::default();
    let plan = plan_fixture(&four_frame, &options);
    assert_eq!(
        plan.frame_candidate_count(),
        4,
        "test fixture must exercise the former total frame-count gate"
    );
    decode_frames_from_plan_on_pool(&four_frame, &options, &plan)
        .expect("fourth multiref frame should decode after widening NumTotalRefs support");
}

#[test]
fn multiref_runtime_does_not_preflight_future_ivf_records_after_decodable_fourth_frame() {
    let future_state = append_future_state_record_after_fourth_multiref_candidate();
    let options = DecodeOptions::default();
    let plan = plan_fixture(&future_state, &options);
    assert_eq!(
        plan.frame_candidate_count(),
        4,
        "test fixture keeps the malformed state-only IVF record after the fourth candidate"
    );
    decode_frames_from_plan_on_pool(&future_state, &options, &plan).expect(
        "malformed state-only IVF record after the fourth candidate should not be preflighted",
    );
}

#[test]
fn multiref_runtime_enforces_cumulative_output_frame_limit() {
    let options = DecodeOptions::default().with_limits(
        DecodeOptions::default()
            .limits()
            .with_max_output_frames(DecodeLimitThreshold::Max(2)),
    );
    let Err(error) = decode_fixture_with_options(MULTIREF_FIXTURE, &options) else {
        panic!("three-frame multiref fixture must exceed max_output_frames=2");
    };
    let DecodeError::Limit { source } = error else {
        panic!("expected max_output_frames resource-limit error");
    };
    assert_eq!(source.name(), DecodeLimitName::MaxOutputFrames);
    let check = source.check().expect("limit failure carries check");
    assert_eq!(check.actual(), 3);
    assert_eq!(check.threshold(), DecodeLimitThreshold::Max(2));
}

#[test]
fn multiref_runtime_enforces_cumulative_reference_store_byte_limit() {
    let options = DecodeOptions::default().with_limits(
        DecodeOptions::default()
            .limits()
            .with_max_reference_store_bytes(DecodeLimitThreshold::Max(12_288)),
    );
    let Err(error) = decode_fixture_with_options(MULTIREF_FIXTURE, &options) else {
        panic!("three-frame multiref fixture must exceed two retained frame byte budget");
    };
    let DecodeError::Limit { source } = error else {
        panic!("expected max_reference_store_bytes resource-limit error");
    };
    assert_eq!(source.name(), DecodeLimitName::MaxReferenceStoreBytes);
    let check = source.check().expect("limit failure carries check");
    assert_eq!(check.actual(), 18_432);
    assert_eq!(check.threshold(), DecodeLimitThreshold::Max(12_288));
}

#[test]
fn multiref_frame2_reads_retained_inter_frame_not_key() {
    let frames = decode_fixture(MULTIREF_FIXTURE);
    let key = frames[0].frame();
    let inter1 = frames[1].frame();
    let inter2 = frames[2].frame();
    assert_eq!(
        inter2.y().samples(),
        inter1.y().samples(),
        "frame 2 luma must equal the retained frame 1 (slot 1)"
    );
    assert_eq!(
        inter2.u().unwrap().samples(),
        inter1.u().unwrap().samples(),
        "frame 2 U must equal the retained frame 1 (slot 1)"
    );
    assert_eq!(
        inter2.v().unwrap().samples(),
        inter1.v().unwrap().samples(),
        "frame 2 V must equal the retained frame 1 (slot 1)"
    );
    assert_ne!(
        inter2.y().samples(),
        key.y().samples(),
        "frame 2 luma must DIFFER from the key (slot 0) — proving it read slot 1, not slot 0"
    );
}

#[test]
fn multiref_fixture_per_frame_hash_is_stable() {
    let frames = decode_fixture(MULTIREF_FIXTURE);
    let hashes = frame_hashes(&frames);
    assert_eq!(
        hashes[0], "ce9c46b1078b9dd593254837ead7dcd6cee8b3ec6cc3c7d34f54fb08df703979",
        "multi-reference key-frame hash"
    );
    assert_eq!(
        hashes[1], "7dad863f3e72b5785012a4e0497e9eb0eab98281bec147f7fb81240aa5116e1b",
        "multi-reference frame-1 hash"
    );
    assert_eq!(
        hashes[2], "7dad863f3e72b5785012a4e0497e9eb0eab98281bec147f7fb81240aa5116e1b",
        "multi-reference frame-2 hash (== retained frame 1)"
    );
    assert_eq!(hashes[1], hashes[2], "frame 2 == retained frame 1");
    assert_ne!(hashes[0], hashes[1], "the inter frames differ from the key");
}

#[test]
fn compound_average_fixture_decodes_three_frames_bit_exact() {
    let frames = decode_fixture(COMPOUND_AVERAGE_FIXTURE);
    assert_eq!(
        frames.len(),
        3,
        "the stream decodes a key frame + two inter frames"
    );
    assert_yuv420_8bit_frames(&frames, 64, 64);

    let frame0 = frames[0].frame();
    let frame1 = frames[1].frame();
    let frame2 = frames[2].frame();
    assert_rounded_average(
        frame0.y().samples(),
        frame1.y().samples(),
        frame2.y().samples(),
    );
    assert_rounded_average(
        frame0.u().unwrap().samples(),
        frame1.u().unwrap().samples(),
        frame2.u().unwrap().samples(),
    );
    assert_rounded_average(
        frame0.v().unwrap().samples(),
        frame1.v().unwrap().samples(),
        frame2.v().unwrap().samples(),
    );
    assert_ne!(
        frame2.y().samples(),
        frame0.y().samples(),
        "compound frame differs from ref 0"
    );
    assert_ne!(
        frame2.y().samples(),
        frame1.y().samples(),
        "compound frame differs from ref 1"
    );
}

#[test]
fn compound_average_fixture_per_frame_hash_is_stable() {
    let frames = decode_fixture(COMPOUND_AVERAGE_FIXTURE);
    let hashes = frame_hashes(&frames);
    assert_eq!(
        hashes,
        [
            "1a1ba40dd0e16691bef8752aa946e3a56a6c76730e08d9a662cd8844da5855d1",
            "0024c5dc9c6fdd85a3f231bc64f6d6668231dc963fb00d16e841a7f525d0b0d2",
            "c00f8963a73155a6c970e8a025332b0865a728c2b5915ab1ad75051f47a35d9e",
        ],
        "compound-average per-frame hashes"
    );
}

#[test]
fn opfl_refine_all_fixture_decodes_eight_frames_bit_exact() {
    let frames = decode_fixture(OPFL_REFINE_ALL_FIXTURE);
    assert_eq!(frames.len(), 8, "the reordered stream outputs eight frames");
    assert_yuv420_8bit_frames(&frames, 64, 64);
    assert_eq!(
        frame_hashes(&frames),
        [
            "ac6643e7adeb891d3474a24e94643a757f142e4f0a22e30b3c8d6a9d22b9fa1e",
            "c0331ed45cab6459a7cf12b8031782313bc00a23258fcbe56ff4f9b6a30345ea",
            "1ed7b6b97aa3432ad7c0c7038690d2ca3afad49d1f608a28c588039fb396ea68",
            "d04c1bb49fa6eeca21ec80ee80ecc35c21d7bccef31424c570411ad62ed58c5b",
            "ab4dd3757f74f9a9334ecaa80ce6767cee6511b802b10501b18852c4ff5e1bc9",
            "84552d55e0a7556120e4822fcb25e3fa0f10daa384c37749466bbd4716417381",
            "6b08ad26c8a8109a0ed5883aaef81c1a1e09922130c4cb775725322500d8661c",
            "eda392e6f6eff8edb7d11bc629463ef5e44bcfa52d3f6a825fd9c7133857aebc",
        ],
        "frame hashes pinned from byte-identical isolated/current AVM output"
    );
}

#[test]
fn opfl_refine_all_fixture_rejects_truncated_payload() {
    let truncated = &OPFL_REFINE_ALL_FIXTURE[..OPFL_REFINE_ALL_FIXTURE.len() - 1];
    let error = decode_context()
        .plan_bytes(truncated, DecodeOptions::default())
        .expect_err("truncating the final coded frame must fail closed");
    assert!(matches!(error, DecodeError::MalformedSource { .. }));
}

fn assert_rounded_average(ref0: &[u8], ref1: &[u8], compound: &[u8]) {
    assert_eq!(ref0.len(), ref1.len(), "reference plane lengths");
    assert_eq!(ref0.len(), compound.len(), "compound plane length");
    for (index, ((&a, &b), &actual)) in ref0
        .iter()
        .zip(ref1.iter())
        .zip(compound.iter())
        .enumerate()
    {
        let expected = ((u16::from(a) + u16::from(b) + 1) >> 1) as u8;
        assert_eq!(
            actual, expected,
            "compound sample {index}: rounded average of refs"
        );
    }
}

#[test]
fn choose_primary_ref_frame_skips_non_inter_slots() {
    use super::cross_frame::choose_primary_secondary_ref_frame as choose;
    const PRIMARY_REF_NONE: u8 = 7;
    let ref_frame_idx = [0u32];
    let is_inter = [false, false];
    let base_q = [70u32, 0];
    let oh = [0u32, 0];
    let w = [64u32, 0];
    let h = [64u32, 0];
    assert_eq!(
        choose(
            Some(false),
            Some(8),
            &ref_frame_idx,
            &is_inter,
            &base_q,
            &oh,
            &w,
            &h,
            70,
            1
        )
        .0,
        PRIMARY_REF_NONE,
        "a KEY-only reference history resolves CHOOSE to PRIMARY_REF_NONE"
    );
    let is_inter = [true, false];
    assert_eq!(
        choose(
            Some(false),
            Some(8),
            &ref_frame_idx,
            &is_inter,
            &base_q,
            &oh,
            &w,
            &h,
            70,
            1
        )
        .0,
        0,
        "an INTER reference resolves CHOOSE to its ref_frame_idx index"
    );
}

#[test]
fn choose_primary_ref_frame_ranks_two_inter_slots_by_qp_diff() {
    use super::cross_frame::choose_primary_secondary_ref_frame as choose;
    let ref_frame_idx = [0u32, 1];
    let is_inter = [true, true];
    let base_q = [70u32, 109];
    let oh = [1u32, 2];
    let w = [64u32, 64];
    let h = [64u32, 64];
    let (primary, secondary) = choose(
        Some(false),
        Some(8),
        &ref_frame_idx,
        &is_inter,
        &base_q,
        &oh,
        &w,
        &h,
        100,
        3,
    );
    assert_eq!(
        primary, 1,
        "the smaller |RefBaseQIdx - base_q_idx| (slot 1) is the derived primary"
    );
    assert_eq!(
        secondary, 0,
        "the other inter candidate (slot 0) is the derived secondary"
    );
}

#[test]
fn resolve_cdf_load_models_choose_resolution_and_load_decision() {
    use super::cross_frame::{ResolvedCdfLoad, resolve_cdf_load as resolve};
    let ref_frame_idx = [0u32, 1];
    let is_inter = [false, true]; // slot 0 key, slot 1 inter
    let base_q = [70u32, 109];
    let oh = [0u32, 1];
    let w = [64u32, 64];
    let h = [64u32, 64];

    let load = resolve(
        Some(false),
        Some(8),     // PRIMARY_REF_CHOOSE
        Some(false), // cross-frame init enabled
        &ref_frame_idx,
        &is_inter,
        &base_q,
        &oh,
        &w,
        &h,
        130,   // current base_q_idx
        2,     // current order hint
        false, // enable_avg_cdf
        1,     // avg_cdf_type
    );
    assert!(
        matches!(
            load,
            ResolvedCdfLoad::LoadSlot {
                primary: 1,
                blend: None
            }
        ),
        "CHOOSE resolves to the inter slot 1 -> load_cdfs(ref_frame_idx[1] == slot 1), no blend"
    );

    let key_only_is_inter = [false, false];
    let load = resolve(
        Some(false),
        Some(8),
        Some(false),
        &[0u32],
        &key_only_is_inter,
        &base_q,
        &oh,
        &w,
        &h,
        130,
        2,
        false, // enable_avg_cdf
        1,     // avg_cdf_type
    );
    assert!(
        matches!(load, ResolvedCdfLoad::Default),
        "CHOOSE over a KEY-only history resolves to PRIMARY_REF_NONE -> Default (no load)"
    );

    let load = resolve(
        Some(false),
        Some(8),
        Some(true), // disable_cross_frame_cdf_init == 1
        &ref_frame_idx,
        &is_inter,
        &base_q,
        &oh,
        &w,
        &h,
        130,
        2,
        false, // enable_avg_cdf
        1,     // avg_cdf_type
    );
    assert!(
        matches!(load, ResolvedCdfLoad::Default),
        "disable_cross_frame_cdf_init == 1 -> Default (init_non_coeff_cdfs)"
    );

    let load = resolve(
        Some(true),
        Some(1), // primary_ref_frame == 1 (ref_frame_idx[1] == slot 1)
        Some(false),
        &ref_frame_idx,
        &is_inter,
        &base_q,
        &oh,
        &w,
        &h,
        130,
        2,
        false, // enable_avg_cdf
        1,     // avg_cdf_type
    );
    assert!(
        matches!(load, ResolvedCdfLoad::LoadSlot { primary: 1, .. }),
        "an explicit primary_ref_frame loads ref_frame_idx[primary_ref_frame]"
    );

    let load = resolve(
        Some(true),
        Some(7),
        Some(false),
        &ref_frame_idx,
        &is_inter,
        &base_q,
        &oh,
        &w,
        &h,
        130,
        2,
        false, // enable_avg_cdf
        1,     // avg_cdf_type
    );
    assert!(
        matches!(load, ResolvedCdfLoad::Default),
        "explicit PRIMARY_REF_NONE -> Default"
    );
}

#[test]
fn resolve_cdf_load_signal_primary_overrides_ranking_even_with_no_inter_candidate() {
    use super::cross_frame::{ResolvedCdfLoad, resolve_cdf_load as resolve};
    let ref_frame_idx = [0u32];
    let is_inter = [false]; // KEY-only history: no inter ranking candidate
    let base_q = [70u32];
    let oh = [0u32];
    let w = [64u32];
    let h = [64u32];
    let load = resolve(
        Some(true), // signal_primary_ref_frame == 1
        Some(0),    // signalled primary_ref_frame 0 (the KEY slot)
        Some(false),
        &ref_frame_idx,
        &is_inter,
        &base_q,
        &oh,
        &w,
        &h,
        130,
        2,
        false, // enable_avg_cdf
        1,     // avg_cdf_type
    );
    assert!(
        matches!(load, ResolvedCdfLoad::LoadSlot { primary: 0, .. }),
        "a signalled primary overrides the (NONE) ranking and loads slot 0"
    );
}

#[test]
fn resolve_cdf_load_reports_blend_slot_only_when_a_secondary_exists() {
    use super::cross_frame::{ResolvedCdfLoad, resolve_cdf_load as resolve};
    let base_q = [70u32, 109];
    let oh = [0u32, 1];
    let w = [64u32, 64];
    let h = [64u32, 64];
    let load = resolve(
        Some(false),
        Some(8),
        Some(false),
        &[0u32, 1],
        &[true, true],
        &base_q,
        &oh,
        &w,
        &h,
        70,
        2,
        true, // enable_avg_cdf
        0,    // avg_cdf_type
    );
    assert!(
        matches!(
            load,
            ResolvedCdfLoad::LoadSlot {
                primary: 0,
                blend: Some(1)
            }
        ),
        "two inter candidates -> primary slot 0, blend slot 1 (the derivedSecondary)"
    );
    let load = resolve(
        Some(false),
        Some(8),
        Some(false),
        &[0u32, 1],
        &[false, true],
        &base_q,
        &oh,
        &w,
        &h,
        70,
        2,
        true,
        0,
    );
    assert!(
        matches!(
            load,
            ResolvedCdfLoad::LoadSlot {
                primary: 1,
                blend: None
            }
        ),
        "one inter candidate -> blendFrame NONE -> no blend"
    );
    let load = resolve(
        Some(true),
        Some(0),
        Some(false),
        &[0u32],
        &[true],
        &[70u32],
        &[0u32],
        &[64u32],
        &[64u32],
        70,
        2,
        true,
        0,
    );
    assert!(
        matches!(
            load,
            ResolvedCdfLoad::LoadSlot {
                primary: 0,
                blend: None
            }
        ),
        "a signalled primary == the sole inter ref has no secondary -> blend None, not rejected"
    );
}

#[test]
fn resolve_cdf_load_rejects_out_of_range_signalled_primary() {
    use super::cross_frame::{ResolvedCdfLoad, resolve_cdf_load as resolve};
    for (disable_cross_frame_cdf_init, reference_is_inter) in [
        (Some(false), true),
        (Some(true), true),
        (Some(false), false),
    ] {
        let load = resolve(
            Some(true),
            Some(6),
            disable_cross_frame_cdf_init,
            &[0u32], // one reference: index 6 is out of bounds
            &[reference_is_inter],
            &[70u32],
            &[0u32],
            &[64u32],
            &[64u32],
            70,
            2,
            false,
            1,
        );
        assert!(
            matches!(
                load,
                ResolvedCdfLoad::OutOfRangePrimary {
                    index: 6,
                    reference_count: 1
                }
            ),
            "a signalled primary >= NumTotalRefs is rejected before default CDF selection"
        );
    }
}

#[test]
fn effective_quantizer_deltas_include_frame_and_sequence_offsets() {
    let (mut sequence, mut quantization) =
        fixture_sequence_and_quantization(TWO_FRAME_RESIDUAL_FIXTURE);
    let tq = sequence
        .transform_quant_entropy
        .as_mut()
        .expect("fixture sequence has transform/quant/entropy config");

    tq.equal_ac_dc_q = false;
    tq.base_y_dc_delta_q = 23;
    tq.base_uv_dc_delta_q = 23;
    tq.base_uv_ac_delta_q = 23;
    quantization.delta_q_y_dc = 0;
    quantization.delta_q_u_dc = 0;
    quantization.delta_q_u_ac = 0;
    quantization.delta_q_v_dc = 0;
    quantization.delta_q_v_ac = 0;
    let deltas = super::effective_quantizer_deltas(&sequence, &quantization)
        .expect("fixture has transform/quant/entropy config");
    assert_eq!(
        (
            deltas.y_dc,
            deltas.u_dc,
            deltas.v_dc,
            deltas.u_ac,
            deltas.v_ac
        ),
        (0, 0, 0, 0, 0)
    );

    quantization.delta_q_y_dc = 1;
    let deltas = super::effective_quantizer_deltas(&sequence, &quantization)
        .expect("fixture has transform/quant/entropy config");
    assert_eq!(deltas.y_dc, 1);
    quantization.delta_q_y_dc = 0;

    sequence
        .transform_quant_entropy
        .as_mut()
        .expect("sequence config")
        .base_uv_ac_delta_q = 24;
    let deltas = super::effective_quantizer_deltas(&sequence, &quantization)
        .expect("fixture has transform/quant/entropy config");
    assert_eq!((deltas.u_ac, deltas.v_ac), (1, 1));
    quantization.delta_q_u_ac = -1;
    quantization.delta_q_v_ac = -1;
    let deltas = super::effective_quantizer_deltas(&sequence, &quantization)
        .expect("fixture has transform/quant/entropy config");
    assert_eq!(
        (
            deltas.y_dc,
            deltas.u_dc,
            deltas.v_dc,
            deltas.u_ac,
            deltas.v_ac
        ),
        (0, 0, 0, 0, 0)
    );

    sequence.general.chroma_format_idc = ChromaFormatIdc::Monochrome;
    quantization.delta_q_u_dc = 5;
    quantization.delta_q_v_dc = -7;
    quantization.delta_q_u_ac = 11;
    quantization.delta_q_v_ac = -13;
    let deltas = super::effective_quantizer_deltas(&sequence, &quantization)
        .expect("fixture has transform/quant/entropy config");
    assert_eq!(
        (
            deltas.y_dc,
            deltas.u_dc,
            deltas.v_dc,
            deltas.u_ac,
            deltas.v_ac
        ),
        (0, 0, 0, 0, 0)
    );

    sequence.transform_quant_entropy = None;
    assert!(matches!(
        super::frame_walk::required_inter_quantizer_deltas(&sequence, &quantization),
        Err(DecodeError::HeaderState {
            source: crate::error::DecodeHeaderStateError::MissingSequenceTransformQuantEntropy,
        })
    ));
}

#[test]
fn tip_output_disables_saved_cdf_blending() {
    assert!(super::cdf_blending_enabled(true, None));
    assert!(super::cdf_blending_enabled(true, Some(TipFrameMode::AsRef)));
    assert!(!super::cdf_blending_enabled(
        true,
        Some(TipFrameMode::AsOutput)
    ));
}

#[test]
fn tip_output_validation_accepts_leading_and_regular_obus() {
    decode_context().pool().install(|| {
        let (_sequence, mut core, _offset) =
            parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).unwrap();
        core.status = FrameHeaderParseStatus::InterHeaderComplete;
        core.frame_is_intra = Some(false);
        core.inter
            .as_mut()
            .expect("fixture has inter control")
            .tip_frame_mode = Some(TipFrameMode::AsOutput);

        for obu_type in [
            splot_core::types::ObuType::LeadingTip,
            splot_core::types::ObuType::RegularTip,
        ] {
            core.obu_type = obu_type;
            super::validate_tip_output_frame_core(&core).unwrap();
        }
    });
}

#[test]
fn tip_output_quantization_uses_nearest_valid_reference_slots() {
    decode_context().pool().install(|| {
        let (mut sequence, mut core, offset) =
            parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).unwrap();
        sequence
            .inter
            .as_mut()
            .expect("fixture has inter sequence config")
            .enable_tip_explicit_qp = false;
        core.order_hint_lsb = Some(10);
        core.order_hint = Some(10);
        core.quantization_params = None;
        core.inter
            .as_mut()
            .expect("fixture has inter control")
            .ref_frame_idx = [0, 1, 2, 3].into_iter().collect();

        let mut reference = super::InterReferenceState::<u8>::empty().unwrap();
        reference.ref_valid = vec![true; 4];
        reference.ref_order_hint = vec![6, 9, 12, 15];
        reference.ref_base_q_idx = vec![50, 101, 104, 200];
        reference.ref_delta_q_u_ac = vec![20, -3, 4, 40];
        reference.ref_delta_q_v_ac = vec![20, -5, -2, 40];

        super::infer_tip_output_quantization(&mut core, &sequence, &reference, offset).unwrap();

        assert_eq!(
            core.quantization_params,
            Some(QuantizationParams::inferred_tip(103, 1, -3))
        );
    });
}

#[test]
fn compound_is_joint_context_uses_strict_same_side_signs() {
    let ctx = compound_is_joint_context_from_order_hints;

    assert_eq!(ctx(10, 10, 10), 0, "zero/zero distances are not same-side");
    assert_eq!(ctx(9, 9, 10), 1, "both past references are same-side");
    assert_eq!(ctx(11, 11, 10), 1, "both future references are same-side");
    assert_eq!(
        ctx(9, 11, 10),
        0,
        "opposite-side equal-distance references stay context 0"
    );
    assert_eq!(
        ctx(9, 12, 10),
        1,
        "opposite-side unequal-distance references still use context 1"
    );
    assert_eq!(
        ctx(-1, 0, 127),
        1,
        "one restricted reference selects context 1 even at equal distance"
    );
}

#[test]
fn compound_is_joint_context_uses_selected_ranked_pair() {
    let ref_frame_idx = [2, 0, 1];
    let ref_order_hint = [9, 10, 11];

    assert_eq!(
        compound_is_joint_context(&ref_frame_idx, &ref_order_hint, (0, 1), 10,).unwrap(),
        0,
        "selected future/past references have equal distance"
    );
    assert_eq!(
        compound_is_joint_context(&ref_frame_idx, &ref_order_hint, (1, 2), 10,).unwrap(),
        1,
        "selected past/current references have unequal distance"
    );
    assert_eq!(
        compound_is_joint_context(&[0, 1], &[u32::MAX, 0], (0, 1), 127).unwrap(),
        1,
        "raw restricted order hints retain the restricted-reference term"
    );
}

#[test]
fn interp_filter_no_neighbour_context_accounts_for_compound_second_ref() {
    assert_eq!(interp_filter_no_neighbour_ctx(false), 3);
    assert_eq!(interp_filter_no_neighbour_ctx(true), 7);
}

#[test]
fn tip_mode_gate_follows_mi_size_not_has_chroma() {
    assert!(tip_allowed_for_block_indices(
        false, false, false, BLOCK_8X8, BLOCK_8X8, 2, 2
    ));
    assert!(!tip_allowed_for_block_indices(
        true, false, false, BLOCK_8X8, BLOCK_8X8, 2, 2
    ));
    assert!(!tip_allowed_for_block_indices(
        false, true, false, BLOCK_8X8, BLOCK_8X8, 2, 2
    ));
    assert!(!tip_allowed_for_block_indices(
        false, false, true, BLOCK_8X8, BLOCK_8X8, 2, 2
    ));
    assert!(!tip_allowed_for_block_indices(
        false,
        false,
        false,
        BLOCK_8X8,
        BLOCK_8X8 + 1,
        2,
        2
    ));
    assert!(!tip_allowed_for_block_indices(
        false, false, false, BLOCK_8X8, BLOCK_8X8, 1, 2
    ));
}

#[test]
fn tile_neighbour_availability_is_tile_bounded() {
    assert_eq!(tile_neighbour_availability(8, 16, 8, 16), (false, false));
    assert_eq!(tile_neighbour_availability(9, 16, 8, 16), (true, false));
    assert_eq!(tile_neighbour_availability(8, 17, 8, 16), (false, true));
    assert_eq!(tile_neighbour_availability(9, 17, 8, 16), (true, true));
}

#[test]
fn floor_log2_matches_msb_index() {
    use super::cross_frame::floor_log2;
    assert_eq!(floor_log2(0), 0);
    assert_eq!(floor_log2(1), 0);
    assert_eq!(floor_log2(2), 1);
    assert_eq!(floor_log2(64 * 64), 12);
    assert_eq!(floor_log2(4095), 11);
}
