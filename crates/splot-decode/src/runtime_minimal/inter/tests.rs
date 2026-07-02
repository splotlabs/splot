// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_parallel::ThreadCount;

use splot_core::headers::frame::{FrameHeaderParseStatus, QuantizationParams};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SequenceHeader};
use splot_core::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};
use splot_core::span::ByteOffset;
use splot_core::stream::{
    ParsedBitstream, ParsedIvfBitstream, ParsedIvfFrame, parse_bitstream_partial,
};
use splot_recon::{
    BitDepth, DecodedFrameHashInput, LoopRestorationSource, PixelFormat, PlaneId, PlaneSize,
    ReconError,
};

use super::super::{MinimalRuntimeFrame, decode_minimal_frames_from_plan};
use super::block::interp_filter_no_neighbour_ctx;
use super::compound_is_joint_context_from_order_hints;
use super::test_support::{
    UnsupportedFeatureExpectation, assert_unsupported_feature, fixture_sequence_and_key_core,
};
use crate::error::{DecodeError, Result};
use crate::tile_payload::{
    MinimalRuntimePartitionFrontierError, TilePartitionTraversalError, WienerNsLrSourceBlock,
};
use crate::{
    DecodeContext, DecodeLimitName, DecodeLimitThreshold, DecodeLimits, DecodeOptions,
    DecodeRuntimeConfig, DecodeStreamPlan,
};

const TWO_FRAME_INTER_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-2frame-inter-64x64.ivf");

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

const TWO_FRAME_SUBPEL_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-subpel-inter-64x64.ivf"
);

const TWO_FRAME_RESIDUAL_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-inter-residual-64x64.ivf"
);

const TWO_FRAME_MVSTACK_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-inter-mvstack-64x64.ivf"
);

const MULTI_SB_INTER_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-2sb-inter-128x64-q80.ivf");

const GRID_INTER_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-grid-inter-128x128-q80.ivf");

const MVORDER_INTER_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-inter-mvorder-64x64.ivf"
);

const FLAT_LUMA: u8 = 100;
const FLAT_CHROMA_U: u8 = 120;
const FLAT_CHROMA_V: u8 = 130;

fn decode_context() -> DecodeContext {
    DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context")
}

fn plan_fixture(bytes: &[u8], options: &DecodeOptions) -> DecodeStreamPlan {
    decode_context().plan_bytes(bytes, *options).expect("plan")
}

fn decode_fixture(bytes: &[u8]) -> Vec<MinimalRuntimeFrame> {
    let options = DecodeOptions::default();
    decode_fixture_with_options(bytes, &options).expect("decode")
}

fn decode_fixture_with_options(
    bytes: &[u8],
    options: &DecodeOptions,
) -> Result<Vec<MinimalRuntimeFrame>> {
    let context = decode_context();
    let plan = context.plan_bytes(bytes, *options).expect("plan");
    context
        .pool()
        .install(|| decode_minimal_frames_from_plan(bytes, options, &plan))
}

fn decode_frames_from_plan_on_pool(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
) -> Result<Vec<MinimalRuntimeFrame>> {
    let context = decode_context();
    context
        .pool()
        .install(|| decode_minimal_frames_from_plan(bytes, options, plan))
}

fn decode_frames() -> Vec<MinimalRuntimeFrame> {
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

fn assert_yuv420_8bit_frames(frames: &[MinimalRuntimeFrame], width: usize, height: usize) {
    let visible_size = PlaneSize::new(width, height).expect("valid visible size");
    for (index, output) in frames.iter().enumerate() {
        let frame = output.frame();
        assert_eq!(frame.bit_depth(), BitDepth::Eight, "frame {index}");
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420, "frame {index}");
        assert_eq!(frame.y().visible_size(), visible_size, "frame {index}");
    }
}

fn frame_hashes(frames: &[MinimalRuntimeFrame]) -> Vec<String> {
    frames
        .iter()
        .map(|output| {
            DecodedFrameHashInput::new(output.frame())
                .compute_hash()
                .to_hex()
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

fn decode_inter_blocks_after_quantization_mutation(
    bytes: &[u8],
    mutate: impl FnOnce(&mut QuantizationParams) + Send,
) -> Result<()> {
    let context = decode_context();
    context
        .pool()
        .install(move || decode_inter_blocks_after_quantization_mutation_inner(bytes, mutate))
}

fn decode_inter_blocks_after_quantization_mutation_inner(
    bytes: &[u8],
    mutate: impl FnOnce(&mut QuantizationParams),
) -> Result<()> {
    let options = DecodeOptions::default();
    let plan = plan_fixture(bytes, &options);
    let parsed = parse_ivf_fixture(bytes, "inter");
    let header = parsed.header.expect("fixture carries an IVF header");
    let first_ivf_frame = parsed.frames.first().expect("fixture carries a key frame");
    let [_td_envelope, sequence_envelope, key_envelope] =
        super::super::require_minimal_obu_order(first_ivf_frame.obus.as_slice())?;
    let sequence = super::super::parse_sequence(sequence_envelope)?;

    let mut candidates = plan.frame_candidates_all();
    let key_candidate = candidates.next().expect("fixture has a key candidate");
    let key_frame = super::super::decode_minimal_key_frame(
        bytes,
        &options,
        &plan,
        key_candidate,
        key_envelope,
        &sequence,
        header,
    )?;
    let key_core = super::super::parse_frame_core(key_envelope, &sequence)?;
    let num_ref_frames = usize::from(
        sequence
            .inter
            .as_ref()
            .expect("fixture sequence has inter config")
            .num_ref_frames,
    );
    let mut reference =
        super::super::reference_buffer::RuntimeReferenceBuffer::new(num_ref_frames)?;
    let frames = vec![key_frame];
    reference.update(
        0,
        &super::super::frame_ref_update_from_core(
            &key_core,
            key_envelope.offset,
            frames[0].frame_cdfs.clone(),
            key_core.order_hint_lsb.unwrap_or(0),
        )?,
    );

    let inter_candidate = candidates.next().expect("fixture has an inter candidate");
    let mut next_unvalidated_following_ivf_record = 1;
    let inter_envelope = super::super::following_inter_envelope(
        &parsed,
        inter_candidate,
        &mut next_unvalidated_following_ivf_record,
    )?;
    let (store, meta) = reference.build_store_eight(&frames)?;
    let inter_state = super::InterReferenceState {
        store: &store,
        ref_valid: meta.ref_valid,
        ref_order_hint: meta.ref_order_hint,
        ref_frame_width: meta.ref_frame_width,
        ref_frame_height: meta.ref_frame_height,
        ref_base_q_idx: meta.ref_base_q_idx,
        ref_is_inter: meta.ref_is_inter,
        ref_adapted: meta.ref_adapted,
        lr_frame_filter_class_counts: meta.lr_frame_filter_class_counts,
        ref_frame_cdfs: meta.ref_frame_cdfs,
    };
    let mut core = super::parse_inter_frame_core(inter_envelope, &sequence, &inter_state)?;
    mutate(
        core.quantization_params
            .as_mut()
            .expect("fixture inter core has quantization params"),
    );
    super::validate_inter_frame_core(&core, &sequence, inter_envelope.offset)?;
    let inter = core
        .inter
        .as_ref()
        .expect("fixture inter core has inter control");
    let tail = core
        .inter_tail
        .as_ref()
        .expect("fixture inter core has inter tail");
    assert!(
        !tail.reference_select,
        "helper covers single-reference fixtures"
    );
    let frame_size = core.frame_size.expect("fixture inter core has frame size");
    let mut workspace = crate::runtime_minimal_recon::new_general_intra_workspace::<u8>(
        frame_size.width as usize,
        frame_size.height as usize,
        BitDepth::Eight,
    )?;
    let ref_frame_idx = inter.ref_frame_idx.clone();
    let qindex = core
        .quantization_params
        .expect("fixture inter core has quantization params")
        .base_q_idx;
    let luma_use_tcq = core
        .lossless_info
        .as_ref()
        .is_some_and(|lossless| lossless.allow_tcq);
    let residual_use_ddt = sequence
        .transform_quant_entropy
        .as_ref()
        .is_some_and(|tq| tq.enable_inter_ddt);
    let initial_cdfs =
        super::resolve_initial_frame_cdfs(&core, &sequence, &inter_state, inter_envelope.offset)?;
    super::block::decode_inter_blocks(
        &plan,
        inter_candidate,
        bytes,
        inter_envelope,
        &sequence,
        &core,
        &options,
        inter
            .interpolation_filter
            .expect("fixture has interpolation filter"),
        inter.num_total_refs.expect("fixture has NumTotalRefs") as usize,
        tail.reference_select,
        None,
        sequence
            .inter
            .as_ref()
            .map_or(0, |seq_inter| seq_inter.num_same_ref_compound),
        &ref_frame_idx,
        &inter_state,
        &mut workspace,
        qindex,
        luma_use_tcq,
        residual_use_ddt,
        BitDepth::Eight,
        initial_cdfs,
    )?;
    Ok(())
}

fn unsupported_reason(error: DecodeError) -> &'static str {
    match error {
        DecodeError::UnsupportedFeature { unsupported } => unsupported.reason(),
        _ => panic!("expected unsupported-feature error"),
    }
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

fn ten_bit_frame_hashes(frames: &[MinimalRuntimeFrame]) -> Vec<String> {
    frames
        .iter()
        .map(|output| {
            DecodedFrameHashInput::new(output.frame_ten().expect("10-bit frame"))
                .compute_hash()
                .to_hex()
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
fn simple_path_interintra_fixture_defers_fail_closed() {
    let Err(error) =
        decode_fixture_with_options(SIMPLE_INTERINTRA_10BIT_FIXTURE, &DecodeOptions::default())
    else {
        panic!("the fixture pins the SIMPLE-path interintra defer");
    };
    assert_eq!(unsupported_reason(error), "inter_interintra_unimplemented");
}

#[test]
fn same_ref_compound_fixture_defers_at_the_block_comp_mode_read() {
    let Err(error) =
        decode_fixture_with_options(SAMEREF_COMPOUND_10BIT_FIXTURE, &DecodeOptions::default())
    else {
        panic!("the fixture pins the same-reference compound defer");
    };
    assert_eq!(
        unsupported_reason(error),
        "compound_missing_is_joint_context"
    );
}

#[test]
fn ccso_reference_reuse_inter_fixture_defers_fail_closed() {
    let Err(error) =
        decode_fixture_with_options(CCSO_REUSE_INTER_10BIT_FIXTURE, &DecodeOptions::default())
    else {
        panic!("the fixture pins the CCSO reference-reuse defer");
    };
    assert_eq!(unsupported_reason(error), "inter_ccso_reuse_unimplemented");
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
fn skip_zero_residual_rejects_nonzero_effective_quantizer_deltas() {
    let Err(error) =
        decode_inter_blocks_after_quantization_mutation(TWO_FRAME_RESIDUAL_FIXTURE, |quant| {
            quant.delta_q_y_dc = 1;
        })
    else {
        panic!("skip == 0 residual with non-zero effective deltas must fail closed");
    };
    assert_eq!(
        unsupported_reason(error),
        "inter_block_residual_quantizer_delta"
    );
}

#[test]
fn skip_one_inter_allows_nonzero_effective_quantizer_deltas() {
    decode_inter_blocks_after_quantization_mutation(TWO_FRAME_INTER_FIXTURE, |quant| {
        quant.delta_q_y_dc = 1;
    })
    .expect("skip == 1 reads no residual and must not hit the residual dequant guard");
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

const TEN_BIT_INTRA_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-intra-64x64-10bit.ivf");

fn repack_first_record_with_extra_regular_tile_group(source: &[u8]) -> Vec<u8> {
    let parsed = parse_ivf_fixture(source, "source");
    assert!(!parsed.frames.is_empty());

    let inter_parsed = parse_multiref_fixture();
    assert_eq!(inter_parsed.frames[1].obus.len(), 2);

    let first_inter_td_end = obu_end_in_ivf_payload(&inter_parsed.frames[1], 0);
    let first_inter_payload = inter_parsed.frames[1].frame.payload;

    let mut leading_payload = Vec::new();
    leading_payload.extend_from_slice(parsed.frames[0].frame.payload);
    leading_payload.extend_from_slice(&first_inter_payload[first_inter_td_end..]);

    let mut header = parsed.header.expect("source fixture has an IVF header");
    header.frame_count = 1;

    let mut bytes = Vec::new();
    write_repacked_ivf_header(&mut bytes, &header);
    write_repacked_ivf_frame(&mut bytes, parsed.frames[0].frame.pts, &leading_payload);

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
fn ten_bit_sequence_passes_runtime_storage_gate() {
    let (mut sequence, _) = fixture_sequence_and_quantization(TWO_FRAME_INTER_FIXTURE);
    sequence.general.bit_depth_idc = BitDepthIdc::Ten;
    super::super::ensure_runtime_storage_bit_depth(&sequence, ByteOffset::new(47))
        .expect("10-bit sequence passes the runtime storage bit-depth gate");

    sequence.general.bit_depth_idc = BitDepthIdc::Eight;
    super::super::ensure_runtime_storage_bit_depth(&sequence, ByteOffset::new(47))
        .expect("8-bit sequence passes the runtime storage bit-depth gate");
}

#[test]
fn wienerns_header_status_reports_precise_runtime_frontier() {
    let error = super::super::incomplete_intra_header_error(
        FrameHeaderParseStatus::StoppedBeforeWienerNsFilter {
            feature_id: "AV2-5.18.7-SEGMENTATION-TILING",
        },
        ByteOffset::new(74),
    );
    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("Wiener NS frontier must be an unsupported-feature error");
    };

    assert_eq!(unsupported.reason(), "unsupported_wienerns_filter");
    assert_eq!(unsupported.matrix_row(), "ac0ej3-wienerns-frontier");
    assert_eq!(unsupported.feature_id(), "DECODE-AC0EJ3-WIENERNS-FRONTIER");
    assert_eq!(unsupported.spec_section(), "5.18.7.11");
    assert_eq!(unsupported.byte_offset(), Some(ByteOffset::new(74)));
    assert!(
        unsupported.message().contains("read_wienerns_filter"),
        "message should name the exact unmodeled parser subroutine"
    );
}

#[test]
fn parsed_wienerns_bank_reports_next_runtime_frontier() {
    let error = super::super::wienerns_lr_source_read_runtime_error(ByteOffset::new(74));
    assert_unsupported_feature(
        error,
        "parsed Wiener NS bank frontier",
        UnsupportedFeatureExpectation::at_byte_offset(
            "unsupported_wienerns_lr_source_read",
            "ac0ej3-lr-source-read-frontier",
            "DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER",
            "7.20.2",
            ByteOffset::new(74),
            &[
                "source-bound facts",
                "per-unit selection state",
                "source-read state",
                "Wiener tap",
                "source sample values",
                "§7.20.3 filtering",
            ],
        ),
    );
}

fn wienerns_lr_source_block() -> WienerNsLrSourceBlock {
    WienerNsLrSourceBlock {
        plane: 0,
        row: 0,
        col: 0,
        unit_row: 0,
        unit_col: 0,
        tile_mi_row_start: 0,
        tile_mi_row_end: 4,
        tile_mi_col_start: 0,
        tile_mi_col_end: 4,
        x: 0,
        y: 6,
        width: 4,
        height: 4,
        luma_start_x: 0,
        luma_end_x: 15,
        luma_start_y: 0,
        luma_end_y: 15,
        frame_luma_end_y: 15,
        luma_stripe_start_y: 8,
        luma_stripe_end_y: 10,
    }
}

fn wienerns_lr_source_read_config() -> super::super::WienerNsLrSourceReadConfig {
    super::super::WienerNsLrSourceReadConfig::CONSERVATIVE
}

#[test]
fn wienerns_lr_source_read_frontier_resolves_source_samples() {
    let blocks = [wienerns_lr_source_block()];
    let expected_output_samples = 16;
    let expected_source_reads = expected_output_samples * (1 + 32);

    let frontier = super::super::derive_wienerns_lr_source_read_frontier(
        &blocks,
        ChromaFormatIdc::Yuv420,
        wienerns_lr_source_read_config(),
        ByteOffset::new(74),
        DecodeLimits::unlimited(),
    )
    .expect("source-read frontier");

    assert_eq!(frontier.blocks_resolved, 1);
    assert_eq!(frontier.output_samples_resolved, expected_output_samples);
    assert_eq!(frontier.source_reads_resolved, expected_source_reads);
    assert_eq!(
        frontier.curr_frame_source_reads + frontier.cdef_frame_source_reads,
        expected_source_reads
    );
    assert_eq!(
        frontier.first_sample,
        Some(super::super::WienerNsLrSourceReadSample {
            plane: PlaneId::Y,
            x: 0,
            y: 6,
            source: LoopRestorationSource::CurrFrame,
        })
    );
}

#[test]
fn wienerns_lr_source_read_frontier_includes_chroma_luma_source_reads() {
    let blocks = [WienerNsLrSourceBlock {
        plane: 1,
        ..wienerns_lr_source_block()
    }];
    let expected_output_samples = 16;
    let expected_source_reads = expected_output_samples * (1 + 12 + (1 + 12) * 4);

    let frontier = super::super::derive_wienerns_lr_source_read_frontier(
        &blocks,
        ChromaFormatIdc::Yuv420,
        wienerns_lr_source_read_config(),
        ByteOffset::new(74),
        DecodeLimits::unlimited(),
    )
    .expect("source-read frontier");

    assert_eq!(frontier.blocks_resolved, 1);
    assert_eq!(frontier.output_samples_resolved, expected_output_samples);
    assert_eq!(frontier.source_reads_resolved, expected_source_reads);
    assert_eq!(
        frontier.curr_frame_source_reads + frontier.cdef_frame_source_reads,
        expected_source_reads
    );
    assert_eq!(
        frontier.first_sample,
        Some(super::super::WienerNsLrSourceReadSample {
            plane: PlaneId::U,
            x: 0,
            y: 6,
            source: LoopRestorationSource::CurrFrame,
        })
    );
}

#[test]
fn wienerns_lr_source_read_frontier_failures_stay_structured() {
    let blocks = [WienerNsLrSourceBlock {
        luma_start_x: 32,
        luma_end_x: 31,
        ..wienerns_lr_source_block()
    }];

    let error = super::super::derive_wienerns_lr_source_read_frontier(
        &blocks,
        ChromaFormatIdc::Yuv420,
        wienerns_lr_source_read_config(),
        ByteOffset::new(74),
        DecodeLimits::unlimited(),
    )
    .unwrap_err();

    match error {
        DecodeError::Reconstruction { source } => {
            assert_eq!(
                source,
                ReconError::LoopRestorationSourceInvalidBounds {
                    field: "luma x range",
                }
            );
        }
        _ => panic!("source-read derivation failures must remain structured"),
    }
    assert_eq!(
        blocks[0].luma_start_x, 32,
        "source-read derivation must not mutate retained source-bound facts"
    );
}

#[test]
fn wienerns_lr_source_read_frontier_rejects_monochrome_chroma_plane() {
    let blocks = [WienerNsLrSourceBlock {
        plane: 1,
        ..wienerns_lr_source_block()
    }];

    let error = super::super::derive_wienerns_lr_source_read_frontier(
        &blocks,
        ChromaFormatIdc::Monochrome,
        wienerns_lr_source_read_config(),
        ByteOffset::new(74),
        DecodeLimits::unlimited(),
    )
    .unwrap_err();

    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("monochrome chroma-plane request must be unsupported-feature");
    };
    assert_eq!(
        unsupported.reason(),
        "unsupported_wienerns_lr_source_chroma_plane"
    );
    assert_eq!(unsupported.matrix_row(), "ac0ej3-lr-source-read-frontier");
    assert_eq!(
        unsupported.feature_id(),
        "DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER"
    );
    assert_eq!(unsupported.spec_section(), "7.20.2");
    assert_eq!(unsupported.byte_offset(), Some(ByteOffset::new(74)));
}

#[test]
fn wienerns_lr_source_read_frontier_rejects_unsupported_plane_index() {
    let blocks = [WienerNsLrSourceBlock {
        plane: 3,
        ..wienerns_lr_source_block()
    }];

    let error = super::super::derive_wienerns_lr_source_read_frontier(
        &blocks,
        ChromaFormatIdc::Yuv420,
        wienerns_lr_source_read_config(),
        ByteOffset::new(74),
        DecodeLimits::unlimited(),
    )
    .unwrap_err();

    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("unsupported plane index must be unsupported-feature");
    };
    assert_eq!(unsupported.reason(), "unsupported_wienerns_lr_source_plane");
    assert_eq!(unsupported.matrix_row(), "ac0ej3-lr-source-read-frontier");
    assert_eq!(
        unsupported.feature_id(),
        "DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER"
    );
    assert_eq!(unsupported.spec_section(), "7.20.2");
    assert_eq!(unsupported.byte_offset(), Some(ByteOffset::new(74)));
}

#[test]
fn wienerns_lr_source_read_frontier_limit_errors_stay_limits() {
    let blocks = [wienerns_lr_source_block()];
    let limits = DecodeLimits::unlimited()
        .with_max_loop_restoration_source_reads(DecodeLimitThreshold::Max(527));

    let error = super::super::derive_wienerns_lr_source_read_frontier(
        &blocks,
        ChromaFormatIdc::Yuv420,
        wienerns_lr_source_read_config(),
        ByteOffset::new(74),
        limits,
    )
    .unwrap_err();

    match error {
        DecodeError::Limit { source } => {
            assert_eq!(
                source.name(),
                DecodeLimitName::MaxLoopRestorationSourceReads
            );
            let check = source.check().expect("limit failure carries check");
            assert_eq!(check.threshold(), DecodeLimitThreshold::Max(527));
            assert_eq!(check.actual(), 528);
        }
        _ => panic!("source-read operation budget failures must remain resource-limit diagnostics"),
    }
}

#[test]
fn wienerns_lr_source_read_frontier_does_not_charge_luma_sample_limit() {
    let blocks = [
        wienerns_lr_source_block(),
        WienerNsLrSourceBlock {
            plane: 1,
            ..wienerns_lr_source_block()
        },
    ];
    let limits =
        DecodeLimits::unlimited().with_max_luma_samples_per_frame(DecodeLimitThreshold::Max(0));

    let frontier = super::super::derive_wienerns_lr_source_read_frontier(
        &blocks,
        ChromaFormatIdc::Yuv420,
        wienerns_lr_source_read_config(),
        ByteOffset::new(74),
        limits,
    )
    .expect("source-read frontier");

    assert_eq!(frontier.blocks_resolved, 2);
    assert_eq!(frontier.output_samples_resolved, 32);
    assert_eq!(frontier.source_reads_resolved, 528 + 1040);
}

#[test]
fn wienerns_lr_unit_frontier_limit_errors_stay_limits() {
    let source = DecodeLimits::unlimited()
        .with_max_tile_partition_steps(DecodeLimitThreshold::Max(0))
        .ensure(DecodeLimitName::MaxTilePartitionSteps, 1)
        .unwrap_err();

    let error = super::super::map_wienerns_lr_unit_frontier_error(
        MinimalRuntimePartitionFrontierError::Traversal(TilePartitionTraversalError::Limit(source)),
        ByteOffset::new(74),
    );

    match error {
        DecodeError::Limit { source } => {
            assert_eq!(source.name(), DecodeLimitName::MaxTilePartitionSteps);
        }
        _ => panic!("LR-unit limit failures must remain resource-limit diagnostics"),
    }
}

#[test]
fn non_wienerns_header_status_keeps_generic_incomplete_frontier() {
    let error = super::super::incomplete_intra_header_error(
        FrameHeaderParseStatus::ActivationFieldsOnly,
        ByteOffset::new(74),
    );
    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("incomplete header fallback must be an unsupported-feature error");
    };

    assert_eq!(unsupported.reason(), "incomplete_frame_header");
    assert_eq!(unsupported.matrix_row(), "minimal-decode-tier-contract");
    assert_eq!(
        unsupported.feature_id(),
        "DECODE-MINIMAL-TIER-RUNTIME-SUCCESS"
    );
    assert_eq!(unsupported.spec_section(), "7.1");
    assert_eq!(unsupported.byte_offset(), Some(ByteOffset::new(74)));
}

#[test]
fn cfl_sequence_tool_rejects_before_tile_decode() {
    let (mut sequence, _) = fixture_sequence_and_quantization(TWO_FRAME_INTER_FIXTURE);
    sequence
        .intra
        .as_mut()
        .expect("fixture has sequence intra config")
        .enable_cfl_intra = true;

    let Err(error) = super::super::ensure_sequence_chroma_tools_before_tile_decode(
        &sequence,
        ByteOffset::new(47),
    ) else {
        panic!("CFL-enabled sequence must fail closed before tile mode-info decode");
    };
    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("CFL tool gate must be an unsupported-feature error");
    };
    assert_eq!(unsupported.reason(), "unsupported_cfl_intra");
    assert_eq!(unsupported.matrix_row(), "ac0ej3-sequence-chroma-frontier");
    assert_eq!(
        unsupported.feature_id(),
        "DECODE-AC0EJ3-SEQUENCE-CHROMA-FRONTIER"
    );
    assert_eq!(unsupported.spec_section(), "5.20.5.6");
}

#[test]
fn mhccp_sequence_tool_rejects_before_tile_decode() {
    let (mut sequence, _) = fixture_sequence_and_quantization(TWO_FRAME_INTER_FIXTURE);
    sequence
        .intra
        .as_mut()
        .expect("fixture has sequence intra config")
        .enable_mhccp = true;

    let Err(error) = super::super::ensure_sequence_chroma_tools_before_tile_decode(
        &sequence,
        ByteOffset::new(47),
    ) else {
        panic!("MHCCP-enabled sequence must fail closed before tile mode-info decode");
    };
    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("MHCCP tool gate must be an unsupported-feature error");
    };
    assert_eq!(unsupported.reason(), "unsupported_mhccp");
    assert_eq!(unsupported.matrix_row(), "ac0ej3-sequence-chroma-frontier");
    assert_eq!(
        unsupported.feature_id(),
        "DECODE-AC0EJ3-SEQUENCE-CHROMA-FRONTIER"
    );
    assert_eq!(unsupported.spec_section(), "5.20.5.6");
}

#[test]
fn leading_key_payload_extra_obu_rejected_before_tile_decode() {
    let repacked = repack_first_record_with_extra_regular_tile_group(TEN_BIT_INTRA_FIXTURE);
    let options = DecodeOptions::default();
    let plan = plan_fixture(&repacked, &options);
    assert!(
        plan.obu_count() >= 4,
        "test fixture must keep an extra OBU after the leading key frame"
    );
    let Err(error) = decode_frames_from_plan_on_pool(&repacked, &options, &plan) else {
        panic!("10-bit leading payload with an extra OBU must fail closed");
    };
    assert_eq!(
        unsupported_reason(error),
        "unexpected_leading_obu_after_key"
    );
}

#[test]
fn multiref_runtime_rejects_extra_obu_after_leading_key_payload() {
    let repacked = repack_first_record_with_extra_regular_tile_group(MULTIREF_FIXTURE);
    let options = DecodeOptions::default();
    let plan = plan_fixture(&repacked, &options);
    assert!(
        plan.obu_count() >= 4,
        "test fixture must keep an extra OBU after the leading key frame"
    );
    let Err(error) = decode_frames_from_plan_on_pool(&repacked, &options, &plan) else {
        panic!("extra leading-payload OBU must fail closed before output");
    };
    assert_eq!(
        unsupported_reason(error),
        "unexpected_leading_obu_after_key"
    );
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
fn four_frame_multiref_reaches_too_many_valid_references_gate() {
    let four_frame = append_multiref_third_frame_as_fourth_ivf_record();
    let options = DecodeOptions::default();
    let plan = plan_fixture(&four_frame, &options);
    assert_eq!(
        plan.frame_candidate_count(),
        4,
        "test fixture must exercise the former total frame-count gate"
    );
    let Err(error) = decode_frames_from_plan_on_pool(&four_frame, &options, &plan) else {
        panic!("a fourth multiref frame must still fail closed before output");
    };
    assert_eq!(unsupported_reason(error), "inter_too_many_valid_references");
}

#[test]
fn multiref_runtime_does_not_preflight_future_ivf_records_before_reference_gate() {
    let future_state = append_future_state_record_after_fourth_multiref_candidate();
    let options = DecodeOptions::default();
    let plan = plan_fixture(&future_state, &options);
    assert_eq!(
        plan.frame_candidate_count(),
        4,
        "test fixture keeps the malformed state-only IVF record after the fourth candidate"
    );
    let Err(error) = decode_frames_from_plan_on_pool(&future_state, &options, &plan) else {
        panic!("a fourth multiref frame must still fail closed before output");
    };
    assert_eq!(unsupported_reason(error), "inter_too_many_valid_references");
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
fn multiref_runtime_enforces_cumulative_output_byte_limit() {
    let options = DecodeOptions::default().with_limits(
        DecodeOptions::default()
            .limits()
            .with_max_output_bytes(DecodeLimitThreshold::Max(12_288)),
    );
    let Err(error) = decode_fixture_with_options(MULTIREF_FIXTURE, &options) else {
        panic!("three-frame multiref fixture must exceed two output frame byte budget");
    };
    let DecodeError::Limit { source } = error else {
        panic!("expected max_output_bytes resource-limit error");
    };
    assert_eq!(source.name(), DecodeLimitName::MaxOutputBytes);
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
        "a signalled primary overrides the (NONE) ranking -> LoadSlot(0), so an adapted slot 0 is rejected, not silently decoded from defaults"
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
    let load = resolve(
        Some(true),
        Some(6),
        Some(false),
        &[0u32], // one reference: index 6 is out of bounds
        &[true],
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
        matches!(load, ResolvedCdfLoad::OutOfRangePrimary),
        "a signalled primary >= NumTotalRefs is OutOfRangePrimary (rejected, not Default)"
    );
}

#[test]
fn effective_quantizer_delta_gate_includes_frame_and_sequence_offsets() {
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
    assert!(
        super::effective_quantizer_deltas_are_zero(&sequence, &quantization),
        "raw sequence base delta 23 maps to effective zero"
    );

    quantization.delta_q_y_dc = 1;
    assert!(
        !super::effective_quantizer_deltas_are_zero(&sequence, &quantization),
        "a parsed frame DeltaQYDc would desync zero-delta dequantization"
    );
    quantization.delta_q_y_dc = 0;

    sequence
        .transform_quant_entropy
        .as_mut()
        .expect("sequence config")
        .base_uv_ac_delta_q = 24;
    assert!(
        !super::effective_quantizer_deltas_are_zero(&sequence, &quantization),
        "a non-zero sequence BaseUVAcDeltaQ would desync zero-delta dequantization"
    );
    quantization.delta_q_u_ac = -1;
    quantization.delta_q_v_ac = -1;
    assert!(
        super::effective_quantizer_deltas_are_zero(&sequence, &quantization),
        "parsed frame deltas may cancel sequence base deltas; the gate checks the effective sums"
    );

    sequence.general.chroma_format_idc = ChromaFormatIdc::Monochrome;
    quantization.delta_q_u_dc = 5;
    quantization.delta_q_v_dc = -7;
    quantization.delta_q_u_ac = 11;
    quantization.delta_q_v_ac = -13;
    assert!(
        super::effective_quantizer_deltas_are_zero(&sequence, &quantization),
        "monochrome streams ignore chroma delta sums"
    );
}

#[test]
fn order_hint_history_wrap_guard() {
    use super::cross_frame::order_hint_history_unwrapped as unwrapped;
    assert!(unwrapped(&[true], &[0u32], 0, 5));
    assert!(unwrapped(&[true], &[0u32], 4, 1));
    assert!(unwrapped(&[true], &[0u32], 4, 9));
    assert!(unwrapped(&[true], &[0u32], 4, 15));
    assert!(!unwrapped(&[true], &[15u32], 4, 0));
    assert!(!unwrapped(&[true], &[8u32], 4, 0));
    assert!(unwrapped(&[true], &[7u32], 4, 0));
    assert!(!unwrapped(&[true, true], &[0u32, 12], 4, 1));
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
}

#[test]
fn interp_filter_no_neighbour_context_accounts_for_compound_second_ref() {
    assert_eq!(interp_filter_no_neighbour_ctx(false), 3);
    assert_eq!(interp_filter_no_neighbour_ctx(true), 7);
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
