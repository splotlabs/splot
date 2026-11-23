// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::reference::buffer::RefSlots;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::SefTrailingBits;
use splot_core::write::{BitWriter, write_annexb_obu};
use splot_recon::CurrentFrameSurface;

const BRIDGE_CELU_FIXTURE: &[u8] =
    include_bytes!("../../../../../../tests/conformance/vectors/valid/syn-bridge-celu-64x64.ivf");
const BRU_INACTIVE_FIXTURE: &[u8] = include_bytes!(
    "../../../../../../tests/conformance/vectors/valid/syn-bru-inactive-frame-128x64-q220.ivf"
);
const BRU_INACTIVE_REFRESH_8BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../../../tests/conformance/vectors/valid/syn-bru-inactive-refresh-8bit.ivf"
);
const BRU_INACTIVE_REFRESH_10BIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../../../tests/conformance/vectors/valid/syn-bru-inactive-refresh-10bit.ivf"
);

#[test]
fn frame_decode_geometry_uses_av2_mi_rounding_and_motion_layout() {
    let (sequence, mut core, _) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.frame_size = Some(splot_core::headers::frame::FrameSize::new(51, 51));
    let geometry = super::super::FrameDecodeGeometry::new(
        &core,
        &sequence,
        splot_recon::BitDepth::Eight,
        false,
    )
    .expect("valid geometry");
    assert_eq!(geometry.mi_dimensions(), (14, 14));
    assert_eq!(geometry.motion_layout().width8(), 7);
    assert_eq!(geometry.motion_layout().height8(), 7);
    assert_eq!(geometry.info().coded_luma_size().width(), 51);
    assert_eq!(geometry.info().storage_luma_size().width(), 56);
    assert_eq!(geometry.info().visible_luma_rect().width(), 51);
}

#[test]
fn frame_decode_geometry_maps_impossible_inputs_to_typed_state_errors() {
    type Mutation = fn(&mut SequenceHeader, &mut FrameHeaderCore);
    let (sequence, core, _) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    let cases: [(Mutation, DecodeHeaderStateError); 3] = [
        (
            |_, core| core.frame_size = None,
            DecodeHeaderStateError::MissingFrameSize,
        ),
        (
            |_, core| {
                core.frame_size = Some(splot_core::headers::frame::FrameSize::new(0, 51));
            },
            DecodeHeaderStateError::ZeroFrameSize,
        ),
        (
            |sequence, _| sequence.partition = None,
            DecodeHeaderStateError::IncompleteInterFrameTools,
        ),
    ];

    for (mutate, expected) in cases {
        let mut sequence = sequence.clone();
        let mut core = core.clone();
        mutate(&mut sequence, &mut core);
        let Err(error) = super::super::FrameDecodeGeometry::new(
            &core,
            &sequence,
            splot_recon::BitDepth::Eight,
            false,
        ) else {
            panic!("invalid geometry was accepted");
        };
        assert!(matches!(error, DecodeError::HeaderState { ref source } if *source == expected));
        assert!(crate::DecodeDiagnosticReport::from_decode_error(&error).is_none());
    }
}

#[test]
fn inter_frame_validation_requires_compound_tool_facts() {
    {
        let run = || -> Result<()> {
            let (mut sequence, mut core, _) =
                parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE)?;
            core.inter
                .as_mut()
                .expect("fixture inter core has control region")
                .opfl_refine_type = None;
            let error = super::super::validate_inter_frame_core(&core, &sequence)
                .expect_err("missing optical-flow refinement type must stay fail-closed");
            assert!(matches!(
                error,
                DecodeError::HeaderState {
                    source: DecodeHeaderStateError::IncompleteInterFrameTools
                }
            ));

            core.inter
                .as_mut()
                .expect("fixture inter core has control region")
                .opfl_refine_type = Some(0);
            sequence.inter = None;
            let error = super::super::validate_inter_frame_core(&core, &sequence)
                .expect_err("missing sequence inter tools must stay fail-closed");
            assert!(matches!(
                error,
                DecodeError::HeaderState {
                    source: DecodeHeaderStateError::IncompleteInterFrameTools
                }
            ));
            Ok(())
        };
        run()
    }
    .unwrap();
}

#[test]
fn avm_inter_fixtures_parse_quarter_and_eighth_pel_precision() {
    use splot_core::headers::frame::MvPrecision;

    for (fixture, expected) in [
        (MULTI_TILE_LR_FIXTURE, MvPrecision::QuarterPel),
        (TWO_FRAME_SUBPEL_FIXTURE, MvPrecision::EighthPel),
    ] {
        let (_, core, _) = parse_inter_core_for_validation(fixture).unwrap();
        assert_eq!(
            core.inter.as_ref().and_then(|inter| inter.mv_precision),
            Some(expected)
        );
    }
}

#[test]
fn inter_residual_cctx_pair_mismatch_is_a_typed_state_error() {
    let mut u_coeffs = all_zero_inter_coeff_block();
    u_coeffs.cctx_type = Some(1);
    let residual_blocks = vec![
        super::super::InterResidualBlock {
            plane: PlaneId::U,
            x: 0,
            y: 0,
            tx_size: 0,
            cctx_pair_delta: 1,
            coeffs: u_coeffs,
        },
        super::super::InterResidualBlock {
            plane: PlaneId::V,
            x: 4,
            y: 0,
            tx_size: 0,
            cctx_pair_delta: -1,
            coeffs: all_zero_inter_coeff_block(),
        },
    ];
    let residual = super::super::InterResidual { block_range: 0..2 };
    let mut workspace = crate::pipeline::reconstruct::new_general_intra_workspace::<u8>(
        16,
        16,
        BitDepth::Eight,
        PixelFormat::Yuv420,
    )
    .unwrap();
    let error = super::super::add_inter_residual_to_workspace(
        &mut CurrentFrameSurface::Frame(&mut workspace),
        &residual,
        &residual_blocks,
        &[],
        100,
        false,
        false,
        BitDepth::Eight,
    )
    .expect_err("mismatched CCTX pair");
    assert!(matches!(
        error,
        DecodeError::HeaderState {
            source: DecodeHeaderStateError::MissingInterResidualCctxPair,
        }
    ));
}

#[test]
fn invalid_inter_residual_block_ranges_are_reconstruction_state() {
    let mut workspace = crate::pipeline::reconstruct::new_general_intra_workspace::<u8>(
        4,
        4,
        BitDepth::Eight,
        PixelFormat::Yuv420,
    )
    .unwrap();

    for (start, end) in [(1, 2), (1, 0)] {
        let error = super::super::add_inter_residual_to_workspace(
            &mut CurrentFrameSurface::Frame(&mut workspace),
            &super::super::InterResidual {
                block_range: start..end,
            },
            &[],
            &[],
            0,
            false,
            false,
            BitDepth::Eight,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            DecodeError::HeaderState {
                source: DecodeHeaderStateError::InvalidInterResidualReconstruction,
            }
        ));
    }
}

#[test]
fn invalid_inter_residual_tx_size_is_reconstruction_state() {
    let residual_blocks = [super::super::InterResidualBlock {
        plane: PlaneId::Y,
        x: 0,
        y: 0,
        tx_size: u8::MAX,
        cctx_pair_delta: 0,
        coeffs: all_zero_inter_coeff_block(),
    }];
    let residual = super::super::InterResidual { block_range: 0..1 };
    let mut workspace = crate::pipeline::reconstruct::new_general_intra_workspace::<u8>(
        4,
        4,
        BitDepth::Eight,
        PixelFormat::Yuv420,
    )
    .unwrap();

    let error = super::super::add_inter_residual_to_workspace(
        &mut CurrentFrameSurface::Frame(&mut workspace),
        &residual,
        &residual_blocks,
        &[],
        0,
        false,
        false,
        BitDepth::Eight,
    )
    .expect_err("invalid stored transform size");

    assert!(matches!(
        error,
        DecodeError::HeaderState {
            source: DecodeHeaderStateError::InvalidInterResidualReconstruction,
        }
    ));
}

#[test]
fn invalid_inter_residual_cctx_tx_size_fails_closed() {
    let mut u_coeffs = all_zero_inter_coeff_block();
    u_coeffs.cctx_type = Some(1);
    let residual_blocks = [
        super::super::InterResidualBlock {
            plane: PlaneId::U,
            x: 0,
            y: 0,
            tx_size: u8::MAX,
            cctx_pair_delta: 1,
            coeffs: u_coeffs,
        },
        super::super::InterResidualBlock {
            plane: PlaneId::V,
            x: 0,
            y: 0,
            tx_size: u8::MAX,
            cctx_pair_delta: -1,
            coeffs: all_zero_inter_coeff_block(),
        },
    ];
    let residual = super::super::InterResidual { block_range: 0..2 };
    let mut workspace = crate::pipeline::reconstruct::new_general_intra_workspace::<u8>(
        16,
        16,
        BitDepth::Eight,
        PixelFormat::Yuv420,
    )
    .unwrap();

    let error = super::super::add_inter_residual_to_workspace(
        &mut CurrentFrameSurface::Frame(&mut workspace),
        &residual,
        &residual_blocks,
        &[],
        0,
        false,
        false,
        BitDepth::Eight,
    )
    .expect_err("invalid stored CCTX transform size");

    assert!(matches!(
        error,
        DecodeError::HeaderState {
            source: DecodeHeaderStateError::InvalidInterResidualReconstruction,
        }
    ));
}

fn repack_first_sef_payload(payload: &[u8]) -> (Vec<u8>, usize) {
    let parsed = parse_ivf_fixture(SEF_FAMILIES_FIXTURE, "SEF families");
    let mut header = parsed.header.expect("SEF fixture has an IVF header");
    header.frame_count = u32::try_from(parsed.frames.len()).expect("fixture frame count fits u32");
    let mut bytes = Vec::new();
    write_repacked_ivf_header(&mut bytes, &header);
    let mut replaced_frame_index = None;
    for (frame_index, frame) in parsed.frames.iter().enumerate() {
        let mut frame_writer = BitWriter::new();
        for envelope in &frame.obus {
            let obu_payload = if replaced_frame_index.is_none() && envelope.header.obu_type.is_sef()
            {
                replaced_frame_index = Some(frame_index);
                payload
            } else {
                envelope.payload
            };
            write_annexb_obu(&mut frame_writer, &envelope.header, obu_payload)
                .expect("repack fixture OBU");
        }
        write_repacked_ivf_frame(&mut bytes, frame.frame.pts, &frame_writer.into_bytes());
    }
    (
        bytes,
        replaced_frame_index.expect("SEF fixture contains a SEF OBU"),
    )
}

#[test]
fn missing_inter_header_regions_are_typed_header_state_errors() {
    use DecodeHeaderStateError::{
        IncompleteInterFrame, IncompleteInterFrameTools, MissingDisplayOrderHint, MissingFrameSize,
        MissingInterControlRegion, MissingInterMotionVectorPrecision, MissingInterTail,
        ZeroFrameSize,
    };
    type MutationCase = (fn(&mut FrameHeaderCore), DecodeHeaderStateError);
    let cases: [MutationCase; 10] = [
        (
            |core| core.status = FrameHeaderParseStatus::ActivationFieldsOnly,
            IncompleteInterFrame,
        ),
        (|core| core.inter = None, MissingInterControlRegion),
        (|core| core.inter_tail = None, MissingInterTail),
        (
            |core| core.inter.as_mut().unwrap().interpolation_filter = None,
            DecodeHeaderStateError::MissingInterpolationFilter,
        ),
        (
            |core| core.inter.as_mut().unwrap().mv_precision = None,
            MissingInterMotionVectorPrecision,
        ),
        (
            |core| core.inter.as_mut().unwrap().max_drl_bits_minus_1 = None,
            IncompleteInterFrameTools,
        ),
        (|core| core.order_hint = None, MissingDisplayOrderHint),
        (|core| core.frame_size = None, MissingFrameSize),
        (
            |core| {
                core.frame_size = Some(splot_core::headers::frame::FrameSize::new(0, 64));
            },
            ZeroFrameSize,
        ),
        (
            |core| {
                core.frame_size = Some(splot_core::headers::frame::FrameSize::new(64, 0));
            },
            ZeroFrameSize,
        ),
    ];
    for (mutate, expected) in cases {
        let error = decode_inter_frame_after_core_mutation(TWO_FRAME_INTER_FIXTURE, mutate)
            .expect_err("header state");
        assert!(matches!(error, DecodeError::HeaderState { source } if source == expected));
    }
}

#[test]
fn truncated_inter_header_regions_are_malformed_source_diagnostics() {
    let (_, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    for status in [
        FrameHeaderParseStatus::StoppedInsideInterControl,
        FrameHeaderParseStatus::StoppedInsideFilterParams,
        FrameHeaderParseStatus::StoppedInsideIntraTail,
    ] {
        core.status = status;
        let error = super::super::validate_inter_frame_parse(&core, offset, Some(3))
            .expect_err("truncated frame header");
        let DecodeError::MalformedSource { issue } = &error else {
            panic!("expected malformed source, got {error}");
        };
        assert_eq!(issue.spec_section(), Some("6.2.1"));
        assert_eq!(issue.frame_index(), Some(3));
        let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
            .expect("truncated frame header must remain user-reportable");
        assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
    }
}

#[test]
fn inter_parser_coverage_preserves_feature_id() {
    let (_, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
        feature_id: "AV2-5.18.7-SEGMENTATION-TILING",
    };
    let error = super::super::validate_inter_frame_parse(&core, offset, Some(4))
        .expect_err("inter parser coverage stop");
    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("expected unsupported feature, got {error}");
    };
    assert_eq!(unsupported.reason(), "AV2-5.18.7-SEGMENTATION-TILING");
    assert_eq!(unsupported.spec_section(), "5.18.2");
}

#[test]
fn complete_intra_only_tile_group_is_admitted_for_regular_and_leading_carriers() {
    let offset = ByteOffset::new(74);
    for obu_type in [
        splot_core::types::ObuType::RegularTileGroup,
        splot_core::types::ObuType::LeadingTileGroup,
    ] {
        let (_, mut core) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
        core.obu_type = obu_type;
        core.frame_type = Some(splot_core::headers::frame::FrameType::IntraOnly);
        core.frame_is_intra = Some(true);

        super::super::validate_inter_frame_parse(&core, offset, Some(4))
            .expect("complete intra-only tile-group header");
    }
}

#[test]
fn contradictory_intra_only_tile_group_state_is_typed() {
    type Mutation = fn(&mut FrameHeaderCore);
    let cases: [Mutation; 4] = [
        |core| core.obu_type = splot_core::types::ObuType::ClosedLoopKey,
        |core| core.frame_type = Some(splot_core::headers::frame::FrameType::Inter),
        |core| core.frame_is_intra = Some(false),
        |core| core.frame_is_intra = None,
    ];
    for mutate in cases {
        let (_, mut core) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
        core.obu_type = splot_core::types::ObuType::RegularTileGroup;
        core.frame_type = Some(splot_core::headers::frame::FrameType::IntraOnly);
        core.frame_is_intra = Some(true);
        mutate(&mut core);

        let error = super::super::validate_inter_frame_parse(&core, ByteOffset::new(74), Some(4))
            .expect_err("contradictory intra-only tile-group state");
        assert!(matches!(
            error,
            DecodeError::HeaderState {
                source: DecodeHeaderStateError::InvalidIntraOnlyTileGroupState
            }
        ));
    }
}

#[test]
fn intra_only_tile_group_never_enters_split_inter_walk() {
    let (_, mut core, _) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    {
        assert!(super::super::splittable_inter_frame(
            splot_core::types::ObuType::RegularTileGroup,
            &core,
        ));
        core.status = FrameHeaderParseStatus::IntraHeaderComplete;
        core.frame_type = Some(splot_core::headers::frame::FrameType::IntraOnly);
        core.frame_is_intra = Some(true);
        assert!(!super::super::splittable_inter_frame(
            splot_core::types::ObuType::RegularTileGroup,
            &core,
        ));
    }
}

#[test]
fn shared_reference_sample_failures_are_typed_reference_state_errors() {
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    let Err(error) = super::super::hold_reference_slot(&reference, 0) else {
        panic!("an empty reference slot must fail closed");
    };
    assert!(matches!(
        error,
        DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::MissingFrame { slot: 0 }
        }
    ));

    let Err(error) = super::super::hold_reference_slot(&reference, 1) else {
        panic!("a reference beyond the active store must fail closed");
    };
    assert!(matches!(
        error,
        DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::SlotOutOfRange {
                slot: 1,
                slot_count: 1
            }
        }
    ));

    let frame = crate::test_support::decoded_frame(4, 4);
    let (slot, _writer) = crate::pipeline::inflight::RefFrameSlot::pending(frame.info())
        .expect("pending reference slot");
    reference
        .put(splot_recon::ReferenceSlot::new(0).expect("slot zero"), slot)
        .expect("store pending reference");
    let Err(error) = super::super::hold_reference_slot(&reference, 0) else {
        panic!("an unpublished reference slot must fail closed");
    };
    assert!(matches!(
        error,
        DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::ReferenceSamplesUnavailable { slot: 0 }
        }
    ));
}

#[test]
fn bridge_frame_validation_maps_impossible_state_to_typed_errors() {
    use DecodeHeaderStateError::{
        IncompleteInterFrame, IncompleteInterFrameTools, InvalidInterReferenceMap,
        MissingDisplayOrderHint, MissingFrameSize, MissingInterControlRegion, ZeroFrameSize,
    };
    type MutationCase = (fn(&mut FrameHeaderCore), DecodeHeaderStateError);

    let (_, core, _) =
        parse_inter_core_for_validation(BRIDGE_CELU_FIXTURE).expect("complete bridge core");
    assert_eq!(core.obu_type, splot_core::types::ObuType::BridgeFrame);
    super::super::validate_bridge_frame_core(&core).expect("valid bridge state");

    let cases: [MutationCase; 12] = [
        (
            |core| core.status = FrameHeaderParseStatus::ActivationFieldsOnly,
            IncompleteInterFrame,
        ),
        (|core| core.frame_is_intra = None, IncompleteInterFrame),
        (
            |core| core.show_existing_frame = Some(true),
            IncompleteInterFrame,
        ),
        (|core| core.order_hint = None, MissingDisplayOrderHint),
        (|core| core.frame_size = None, MissingFrameSize),
        (
            |core| {
                core.frame_size = Some(splot_core::headers::frame::FrameSize::new(0, 64));
            },
            ZeroFrameSize,
        ),
        (|core| core.tile_info = None, IncompleteInterFrameTools),
        (
            |core| core.quantization_params = None,
            IncompleteInterFrameTools,
        ),
        (
            |core| core.bridge_film_grain = None,
            IncompleteInterFrameTools,
        ),
        (|core| core.inter = None, MissingInterControlRegion),
        (
            |core| core.bridge_frame_ref_idx = None,
            InvalidInterReferenceMap,
        ),
        (
            |core| core.inter.as_mut().unwrap().num_total_refs = Some(2),
            InvalidInterReferenceMap,
        ),
    ];

    for (mutate, expected) in cases {
        let mut mutated = core.clone();
        mutate(&mut mutated);
        let error = super::super::validate_bridge_frame_core(&mutated)
            .expect_err("impossible bridge state");
        assert!(matches!(error, DecodeError::HeaderState { ref source } if *source == expected));
        assert!(crate::DecodeDiagnosticReport::from_decode_error(&error).is_none());
    }
}

#[test]
fn admitted_reference_samples_survive_a_later_slot_failure() {
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    let frame = crate::test_support::decoded_frame(4, 4);
    let (slot, writer) = crate::pipeline::inflight::RefFrameSlot::pending(frame.info())
        .expect("pending reference slot");
    let progress = slot.progress().expect("pending frame progress");
    progress.begin(&[(0, 4)]).expect("stripe geometry");
    progress.publish(0);
    reference
        .put(splot_recon::ReferenceSlot::new(0).expect("slot zero"), slot)
        .expect("store pending reference");

    let (reference0, reference1) =
        super::super::hold_reference_pair(&reference, 0, None).expect("published samples");
    let info = frame.info();
    let scalings =
        super::super::mv_scaling::PlaneScaling::of_reference_planes(info, info.coded_luma_size());
    let held = super::super::HeldInterBlockReferences {
        reference0: reference0.samples(&scalings),
        reference1: reference1.unwrap_or(reference0).samples(&scalings),
        compound: false,
    };
    drop(writer);

    assert!(
        reference
            .slot(0)
            .and_then(crate::pipeline::inflight::RefFrameSlot::hold_samples)
            .is_none()
    );
    assert_eq!(held.reference0_samples().info(), info);
}

#[test]
fn missing_reference_cdf_context_is_a_typed_reference_state_error() {
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    let failed = super::super::FrameCdfHandle::pending();
    failed.fail();
    reference.ref_frame_cdfs =
        RefSlots::from_iter_checked([Some(failed)]).expect("reference slots fit");

    for slot in [0, 1] {
        let error = reference
            .cdfs_for_slot(slot)
            .expect_err("missing reference CDF context");
        assert!(matches!(
            error,
            DecodeError::ReferenceState {
                source: crate::DecodeReferenceStateError::MissingCdfContext { slot: actual }
            } if actual == slot as usize
        ));
    }
}

#[test]
fn missing_reference_ccso_params_is_a_typed_reference_state_error() {
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_ccso_headers = RefSlots::from_iter_checked([None]).expect("reference slots fit");

    for slot in [0, 1] {
        let error = reference
            .ccso_params_for_slot(slot)
            .expect_err("missing saved CCSO parameters");
        assert!(matches!(
            error,
            DecodeError::ReferenceState {
                source: crate::DecodeReferenceStateError::MissingCcsoParams { slot: actual }
            } if actual == slot as usize
        ));
    }
}

#[test]
fn missing_reference_ccso_plane_is_a_malformed_source_diagnostic() {
    let (_, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    let inter = core.inter.as_mut().expect("inter control");
    inter.num_total_refs = Some(1);
    inter.ref_frame_idx = [0].into_iter().collect();
    let saved_ccso = core.ccso_params.clone().expect("saved CCSO state");
    let reuse_plane = splot_core::headers::frame::CcsoPlaneParams {
        ccso_planes: true,
        reuse_ccso: true,
        sb_reuse_ccso: false,
        ccso_ref_idx: Some(0),
        ccso_bo_only: None,
        ccso_scale_idx: None,
        ccso_quant_idx: None,
        ccso_ext_filter: None,
        ccso_edge_clf: None,
        ccso_max_band_log2: None,
        ccso_offset_idx: splot_core::headers::frame::CcsoOffsets::default(),
    };
    core.ccso_params
        .as_mut()
        .expect("CCSO state")
        .planes
        .push(reuse_plane);
    let mut disabled_ccso = saved_ccso.clone();
    let mut disabled_plane = reuse_plane;
    disabled_plane.ccso_planes = false;
    disabled_plane.reuse_ccso = false;
    disabled_plane.ccso_ref_idx = None;
    disabled_ccso.planes.push(disabled_plane);
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    for (saved, reuse_ccso, sb_reuse_ccso) in [
        (saved_ccso.clone(), true, false),
        (disabled_ccso.clone(), true, false),
        (disabled_ccso, false, true),
    ] {
        let plane = &mut core.ccso_params.as_mut().expect("CCSO state").planes[0];
        plane.reuse_ccso = reuse_ccso;
        plane.sb_reuse_ccso = sb_reuse_ccso;
        let mut saved_header = core.clone();
        saved_header.ccso_params = Some(saved);
        reference.ref_ccso_headers =
            RefSlots::from_iter_checked([Some(std::sync::Arc::new(saved_header))])
                .expect("reference slots fit");
        let error =
            super::super::resolve_ccso_reference_reuse(&mut core, &reference, offset, Some(2))
                .expect_err("missing or disabled saved CCSO plane");
        let DecodeError::MalformedSource { issue } = &error else {
            panic!("expected malformed source, got {error}");
        };
        assert_eq!(issue.spec_section(), Some("6.17.7.8"));
        assert_eq!(issue.offset(), Some(offset));
        assert_eq!(issue.frame_index(), Some(2));
        assert_eq!(
            issue.message(),
            "CCSO reference slot 0 has no saved enabled plane 0; \
             SavedCcsoPlanes must equal 1 when ccso_ref_idx is present"
        );
        let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
            .expect("invalid saved CCSO plane must remain user-reportable");
        assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
    }
}

#[test]
fn ras_missing_reference_map_is_a_typed_header_state_error() {
    let (_, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.obu_type = splot_core::types::ObuType::RasFrame;
    core.inter = None;
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error = super::super::validate_ras_reference_ids(&core, &reference, offset, None)
        .expect_err("RAS reference map");
    assert!(matches!(
        error,
        DecodeError::HeaderState {
            source: DecodeHeaderStateError::MissingInterControlRegion
        }
    ));
}

#[test]
fn truncated_inter_frame_prefix_is_a_malformed_source_diagnostic() {
    const TRUNCATED_PREFIX: &[u8] = &[0];
    let (sequence, _) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
    let parsed = parse_ivf_fixture(TWO_FRAME_INTER_FIXTURE, "inter");
    let mut envelope = parsed.frames[1]
        .obus
        .iter()
        .find(|envelope| envelope.header.obu_type == splot_core::types::ObuType::RegularTileGroup)
        .copied()
        .expect("inter tile group");
    envelope.payload = TRUNCATED_PREFIX;
    envelope.size = u32::from(envelope.header.header_size_bytes) + 1;
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error =
        super::super::parse_inter_frame_activation(envelope, &sequence, &reference, true, Some(1))
            .expect_err("truncated frame-header prefix");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("5.18.2"));
    assert_eq!(issue.offset(), Some(envelope.offset));
    assert_eq!(issue.frame_index(), Some(1));
    assert!(issue.message().contains("unexpected end of input"));
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("truncated frame header must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn missing_tile_group_prefix_is_a_malformed_source_diagnostic() {
    let (sequence, _) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
    let parsed = parse_ivf_fixture(TWO_FRAME_INTER_FIXTURE, "inter");
    let mut envelope = parsed.frames[1]
        .obus
        .iter()
        .find(|envelope| envelope.header.obu_type == splot_core::types::ObuType::RegularTileGroup)
        .copied()
        .expect("inter tile group");
    envelope.payload = &[];
    envelope.size = u32::from(envelope.header.header_size_bytes);
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error =
        super::super::parse_inter_frame_activation(envelope, &sequence, &reference, true, Some(1))
            .expect_err("missing tile-group prefix");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("5.19"));
    assert_eq!(issue.offset(), Some(envelope.offset));
    assert_eq!(issue.frame_index(), Some(1));
    assert_eq!(
        issue.message(),
        "tile group payload ends before is_first_tile_group"
    );
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("missing tile-group prefix must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn malformed_sef_frame_header_is_a_malformed_source_diagnostic() {
    let (bytes, frame_index) = repack_first_sef_payload(&[]);
    let options = DecodeOptions::default();

    let result = decode_frames_from_plan(&bytes, &options);
    let Err(error) = result else {
        panic!("malformed SEF frame header decoded successfully");
    };
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("5.18.2"));
    assert_eq!(issue.frame_index(), Some(frame_index));
    assert!(issue.message().contains("unexpected end of input"));
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("malformed SEF frame header must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn truncated_tip_activation_prefix_is_a_malformed_source_diagnostic() {
    let (sequence, _) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
    let parsed = parse_ivf_fixture(TWO_FRAME_INTER_FIXTURE, "inter");
    let mut envelope = parsed.frames[1]
        .obus
        .iter()
        .find(|envelope| envelope.header.obu_type == splot_core::types::ObuType::RegularTileGroup)
        .copied()
        .expect("inter tile group");
    envelope.header.obu_type = splot_core::types::ObuType::RegularTip;
    envelope.payload = &[];
    envelope.size = u32::from(envelope.header.header_size_bytes);
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error =
        super::super::parse_inter_frame_activation(envelope, &sequence, &reference, true, Some(1))
            .expect_err("truncated TIP activation prefix");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("5.18.2"));
    assert_eq!(issue.offset(), Some(envelope.offset));
    assert_eq!(issue.frame_index(), Some(1));
    assert!(issue.message().contains("unexpected end of input"));
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("truncated TIP activation prefix must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn sef_eof_inside_film_grain_is_a_malformed_source_diagnostic() {
    let (mut sequence, _) = fixture_sequence_and_key_core(SEF_FAMILIES_FIXTURE);
    sequence.film_grain_params_present = Some(true);
    let parsed = parse_ivf_fixture(SEF_FAMILIES_FIXTURE, "SEF families");
    let (frame_index, mut envelope) = parsed
        .frames
        .iter()
        .enumerate()
        .find_map(|(frame_index, frame)| {
            frame
                .obus
                .iter()
                .find(|envelope| envelope.header.obu_type.is_sef())
                .copied()
                .map(|envelope| (frame_index, envelope))
        })
        .expect("SEF OBU");
    let mut payload = BitWriter::new();
    payload.write_uvlc(0).expect("cur_mfh_id");
    payload.write_uvlc(0).expect("seq_header_id");
    payload.write_bits(6, 3).expect("frame_to_show_map_idx");
    payload.write_flag(true).expect("derive_sef_order_hint");
    payload.write_flag(true).expect("apply_grain");
    payload.write_bits(2, 3).expect("fgm_id");
    payload.write_bits(0, 8).expect("partial grain_seed");
    let payload = payload.into_bytes();
    envelope.payload = &payload;
    envelope.size = u32::from(envelope.header.header_size_bytes)
        + u32::try_from(payload.len()).expect("payload length fits u32");
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error = super::super::parse_inter_frame_activation(
        envelope,
        &sequence,
        &reference,
        true,
        Some(frame_index),
    )
    .expect_err("SEF truncated inside film grain");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("5.18.2"));
    assert_eq!(issue.offset(), Some(envelope.offset));
    assert_eq!(issue.frame_index(), Some(frame_index));
    assert_eq!(
        issue.message(),
        "show-existing frame header ends inside film_grain_config()"
    );
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("truncated SEF frame header must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn malformed_sef_trailing_bits_are_a_malformed_source_diagnostic() {
    let mut payload = BitWriter::new();
    payload.write_uvlc(0).expect("cur_mfh_id");
    payload.write_uvlc(0).expect("seq_header_id");
    payload.write_bits(0, 3).expect("frame_to_show_map_idx");
    payload.write_flag(true).expect("derive_sef_order_hint");
    payload.write_bit(1).expect("trailing_one_bit");
    payload.write_bit(1).expect("nonzero trailing_zero_bit");
    let (bytes, frame_index) = repack_first_sef_payload(&payload.into_bytes());
    let options = DecodeOptions::default();

    let result = decode_frames_from_plan(&bytes, &options);
    let Err(error) = result else {
        panic!("SEF with malformed trailing bits decoded successfully");
    };
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(issue.spec_section(), Some("6.2.3"));
    assert_eq!(issue.frame_index(), Some(frame_index));
    assert!(issue.message().contains("trailing_zero_bit"));
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("malformed SEF trailing bits must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn sef_reference_slot_out_of_range_is_a_malformed_source_diagnostic() {
    let (mut sequence, _) = fixture_sequence_and_key_core(SEF_FAMILIES_FIXTURE);
    sequence
        .inter
        .as_mut()
        .expect("sequence inter config")
        .num_ref_frames = 3;
    let order_hint_bits = u32::from(
        sequence
            .inter
            .as_ref()
            .expect("sequence inter config")
            .order_hint_bits,
    );
    let parsed = parse_ivf_fixture(SEF_FAMILIES_FIXTURE, "SEF families");
    let (frame_index, sef_envelope) = parsed
        .frames
        .iter()
        .enumerate()
        .find_map(|(frame_index, frame)| {
            frame
                .obus
                .iter()
                .find(|envelope| envelope.header.obu_type.is_sef())
                .copied()
                .map(|envelope| (frame_index, envelope))
        })
        .expect("SEF OBU");
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_valid =
        RefSlots::from_iter_checked(core::iter::repeat_n(false, 3)).expect("reference slots fit");
    reference.ref_order_hint =
        RefSlots::from_iter_checked(core::iter::repeat_n(0, 3)).expect("reference slots fit");
    for derive_sef_order_hint in [false, true] {
        let mut envelope = sef_envelope;
        let mut payload = BitWriter::new();
        payload.write_uvlc(0).expect("cur_mfh_id");
        payload.write_uvlc(0).expect("seq_header_id");
        payload.write_bits(3, 2).expect("frame_to_show_map_idx");
        payload
            .write_flag(derive_sef_order_hint)
            .expect("derive_sef_order_hint");
        if !derive_sef_order_hint {
            payload
                .write_bits(0, order_hint_bits)
                .expect("display_order_hint");
        }
        payload.write_bit(1).expect("trailing_one_bit");
        let payload = payload.into_bytes();
        envelope.payload = &payload;
        envelope.size = u32::from(envelope.header.header_size_bytes)
            + u32::try_from(payload.len()).expect("payload length fits u32");

        let error = super::super::parse_validated_inter_frame_core_with_mfh(
            envelope,
            &sequence,
            &reference,
            true,
            None,
            Some(frame_index),
        )
        .expect_err("out-of-range SEF reference slot");
        let DecodeError::MalformedSource { issue } = &error else {
            panic!("expected malformed source, got {error}");
        };
        assert_eq!(issue.spec_section(), Some("6.17.2"));
        assert_eq!(issue.frame_index(), Some(frame_index));
        assert_eq!(
            issue.message(),
            "show-existing-frame reference slot 3 is outside the active 3-slot buffer"
        );
    }
}

#[test]
fn empty_sef_trailing_bits_use_payload_conformance_section() {
    let (sequence, _) = fixture_sequence_and_key_core(SEF_FAMILIES_FIXTURE);
    let parsed = parse_ivf_fixture(SEF_FAMILIES_FIXTURE, "SEF families");
    let envelope = parsed
        .frames
        .iter()
        .flat_map(|frame| &frame.obus)
        .find(|envelope| envelope.header.obu_type.is_sef())
        .copied()
        .expect("SEF OBU");
    let num_ref_frames = usize::from(
        sequence
            .inter
            .as_ref()
            .expect("sequence inter config")
            .num_ref_frames,
    );
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_valid = RefSlots::from_iter_checked(core::iter::repeat_n(false, num_ref_frames))
        .expect("reference slots fit");
    reference.ref_order_hint = RefSlots::from_iter_checked(core::iter::repeat_n(0, num_ref_frames))
        .expect("reference slots fit");
    let mut core =
        super::super::parse_inter_frame_activation(envelope, &sequence, &reference, true, Some(2))
            .expect("complete SEF state");
    core.sef_trailing_bits = Some(SefTrailingBits::Empty);

    let error = super::super::validate_sef_frame_core(&core, &reference, envelope.offset, Some(2))
        .expect_err("empty SEF trailing bits");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(issue.spec_section(), Some("6.2.1"));
}

fn tip_output_core_for_validation() -> (FrameHeaderCore, ByteOffset) {
    let (_, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.status = FrameHeaderParseStatus::InterHeaderComplete;
    core.obu_type = splot_core::types::ObuType::RegularTip;
    core.frame_is_intra = Some(false);
    core.inter.as_mut().expect("inter control").tip_frame_mode = Some(TipFrameMode::AsOutput);
    (core, offset)
}

#[test]
fn tip_output_quantization_missing_control_is_a_typed_header_state_error() {
    let (mut sequence, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    sequence
        .inter
        .as_mut()
        .expect("fixture has inter sequence config")
        .enable_tip_explicit_qp = false;
    core.quantization_params = None;
    core.inter = None;
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error =
        super::super::infer_tip_output_quantization(&mut core, &sequence, &reference, offset, None)
            .expect_err("missing TIP inter control");
    assert!(matches!(
        error,
        DecodeError::HeaderState {
            source: DecodeHeaderStateError::MissingInterControlRegion,
        }
    ));
}

#[test]
fn tip_output_without_reference_pair_is_a_malformed_source_diagnostic() {
    let (mut sequence, _) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
    let (mut core, offset) = tip_output_core_for_validation();
    sequence
        .inter
        .as_mut()
        .expect("fixture has inter sequence config")
        .enable_tip_explicit_qp = false;
    core.order_hint_lsb = Some(10);
    core.order_hint = Some(10);
    core.quantization_params = None;
    core.inter.as_mut().expect("inter control").ref_frame_idx = [0, 1].into_iter().collect();
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_valid =
        RefSlots::from_iter_checked(core::iter::repeat_n(true, 2)).expect("reference slots fit");
    reference.ref_order_hint =
        RefSlots::from_iter_checked(core::iter::repeat_n(10, 2)).expect("reference slots fit");

    let error = super::super::infer_tip_output_quantization(
        &mut core,
        &sequence,
        &reference,
        offset,
        Some(6),
    )
    .expect_err("TIP reference pair");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(issue.spec_section(), Some("7.10.1"));
    assert_eq!(issue.frame_index(), Some(6));
    assert!(issue.message().contains("past/future reference pair"));
}

#[test]
fn explicit_qp_tip_output_without_reference_pair_is_malformed_during_admission() {
    let (mut sequence, _) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
    let (mut core, offset) = tip_output_core_for_validation();
    sequence
        .inter
        .as_mut()
        .expect("fixture has inter sequence config")
        .enable_tip_explicit_qp = true;
    core.order_hint_lsb = Some(10);
    core.order_hint = Some(10);
    assert!(core.quantization_params.is_some());
    core.inter.as_mut().expect("inter control").ref_frame_idx = [0, 1].into_iter().collect();
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_valid =
        RefSlots::from_iter_checked(core::iter::repeat_n(true, 2)).expect("reference slots fit");
    reference.ref_order_hint = RefSlots::from_iter_checked([12, 13]).expect("reference slots fit");

    let error = super::super::infer_tip_output_quantization(
        &mut core,
        &sequence,
        &reference,
        offset,
        Some(7),
    )
    .expect_err("TIP reference pair");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(issue.spec_section(), Some("7.10.1"));
    assert_eq!(issue.frame_index(), Some(7));
}

#[test]
fn tip_as_ref_without_reference_pair_is_malformed_during_admission() {
    let (sequence, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.order_hint_lsb = Some(10);
    core.order_hint = Some(10);
    let inter = core.inter.as_mut().expect("inter control");
    inter.tip_frame_mode = Some(TipFrameMode::AsRef);
    inter.ref_frame_idx = [0, 1].into_iter().collect();
    inter.num_total_refs = Some(2);
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_valid =
        RefSlots::from_iter_checked(core::iter::repeat_n(true, 2)).expect("reference slots fit");
    reference.ref_order_hint = RefSlots::from_iter_checked([12, 13]).expect("reference slots fit");

    let error = super::super::validate_and_resolve_inter_frame_core(
        &mut core,
        &sequence,
        &reference,
        offset,
        Some(8),
    )
    .expect_err("TIP reference pair");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(issue.spec_section(), Some("7.10.1"));
    assert_eq!(issue.frame_index(), Some(8));
}

#[test]
fn tip_output_missing_reference_quantizer_is_a_typed_reference_state_error() {
    let (mut sequence, _) = fixture_sequence_and_key_core(TWO_FRAME_INTER_FIXTURE);
    let (mut core, offset) = tip_output_core_for_validation();
    sequence
        .inter
        .as_mut()
        .expect("fixture has inter sequence config")
        .enable_tip_explicit_qp = false;
    core.order_hint_lsb = Some(10);
    core.order_hint = Some(10);
    core.quantization_params = None;
    core.inter.as_mut().expect("inter control").ref_frame_idx = [0, 1].into_iter().collect();
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_valid =
        RefSlots::from_iter_checked(core::iter::repeat_n(true, 2)).expect("reference slots fit");
    reference.ref_order_hint = RefSlots::from_iter_checked([9, 12]).expect("reference slots fit");

    let error =
        super::super::infer_tip_output_quantization(&mut core, &sequence, &reference, offset, None)
            .expect_err("TIP reference quantizer metadata");
    assert!(matches!(
        error,
        DecodeError::ReferenceState {
            source: crate::error::DecodeReferenceStateError::MissingQuantizerMetadata { slot: 0 },
        }
    ));
}

#[test]
fn truncated_tip_output_header_is_a_malformed_source_diagnostic() {
    let (mut core, offset) = tip_output_core_for_validation();
    core.status = FrameHeaderParseStatus::StoppedInsideInterControl;

    let error = super::super::validate_tip_output_frame_parse(&core, offset, Some(3))
        .expect_err("truncated TIP-output header");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(issue.spec_section(), Some("6.2.1"));
    assert_eq!(issue.frame_index(), Some(3));
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("truncated TIP-output header must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn non_output_tip_mode_is_a_malformed_source_diagnostic() {
    let (mut core, offset) = tip_output_core_for_validation();
    core.inter.as_mut().expect("inter control").tip_frame_mode = Some(TipFrameMode::Disabled);

    let error = super::super::validate_tip_output_frame_parse(&core, offset, Some(4))
        .expect_err("non-output TIP mode");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(issue.spec_section(), Some("6.17.2"));
    assert_eq!(issue.frame_index(), Some(4));
}

#[test]
fn single_picture_tip_obu_is_a_malformed_source_diagnostic() {
    let (sequence, _) = fixture_sequence_and_key_core(SINGLE_PICTURE_BRIDGE_FIXTURE);
    assert!(sequence.general.single_picture_header_flag);
    let parsed = parse_ivf_fixture(SINGLE_PICTURE_BRIDGE_FIXTURE, "single-picture bridge");
    let (frame_index, mut envelope) = parsed
        .frames
        .iter()
        .enumerate()
        .find_map(|(frame_index, frame)| {
            frame
                .obus
                .iter()
                .find(|envelope| {
                    envelope.header.obu_type == splot_core::types::ObuType::ClosedLoopKey
                })
                .copied()
                .map(|envelope| (frame_index, envelope))
        })
        .expect("closed-loop-key OBU");
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    reader.read_bit().expect("is_first_tile_group");
    let mut payload = BitWriter::new();
    while reader.remaining_bits() != 0 {
        payload
            .write_bit(reader.read_bit().expect("key payload bit"))
            .expect("TIP payload bit");
    }
    let payload = payload.into_bytes();
    envelope.header.obu_type = splot_core::types::ObuType::RegularTip;
    envelope.payload = &payload;
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    let error = super::super::parse_validated_inter_frame_core_with_mfh(
        envelope,
        &sequence,
        &reference,
        true,
        None,
        Some(frame_index),
    )
    .expect_err("TIP OBU under a single-picture sequence");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(issue.spec_section(), Some("6.17.2"));
    assert_eq!(issue.frame_index(), Some(frame_index));
}

#[test]
fn tip_output_parser_coverage_preserves_feature_id() {
    let (mut core, offset) = tip_output_core_for_validation();
    core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
        feature_id: "AV2-5.18.7-SEGMENTATION-TILING",
    };

    let error = super::super::validate_tip_output_frame_parse(&core, offset, Some(5))
        .expect_err("TIP-output parser coverage stop");
    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("expected unsupported feature, got {error}");
    };
    assert_eq!(unsupported.reason(), "AV2-5.18.7-SEGMENTATION-TILING");
    assert_eq!(unsupported.spec_section(), "5.18.2");
}

#[test]
fn impossible_tip_output_state_is_a_typed_header_state_error() {
    let (mut core, _offset) = tip_output_core_for_validation();
    core.frame_is_intra = Some(true);

    let error = super::super::validate_tip_output_frame_core(&core)
        .expect_err("incomplete TIP-output state");
    assert!(matches!(
        error,
        DecodeError::HeaderState {
            source: DecodeHeaderStateError::IncompleteTipOutput
        }
    ));
}

#[test]
fn impossible_tip_output_runtime_inputs_are_typed_state_errors() {
    type MutationCase = (
        fn(&mut SequenceHeader, &mut FrameHeaderCore),
        DecodeHeaderStateError,
    );
    let (sequence, core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    let cases: [MutationCase; 4] = [
        (
            |_, core| core.frame_size = None,
            DecodeHeaderStateError::MissingFrameSize,
        ),
        (
            |_, core| core.inter = None,
            DecodeHeaderStateError::MissingInterControlRegion,
        ),
        (
            |sequence, _| sequence.partition = None,
            DecodeHeaderStateError::IncompleteInterFrameTools,
        ),
        (
            |_, core| core.order_hint = None,
            DecodeHeaderStateError::MissingDisplayOrderHint,
        ),
    ];

    for (mutate, expected) in cases {
        let mut sequence = sequence.clone();
        let mut core = core.clone();
        mutate(&mut sequence, &mut core);
        let geometry = match super::super::FrameDecodeGeometry::new(
            &core,
            &sequence,
            splot_recon::BitDepth::Eight,
            false,
        ) {
            Ok(geometry) => geometry,
            Err(error) => {
                assert!(matches!(error, DecodeError::HeaderState { source } if source == expected));
                continue;
            }
        };
        let error = super::super::block::tip::reconstruct_output(
            &mut super::super::InterDecodeScratch::default(),
            splot_recon::FramePlaneSamples::default(),
            &sequence,
            &core,
            &reference,
            geometry,
            offset,
        )
        .expect_err("TIP-output runtime state");
        assert!(matches!(error, DecodeError::HeaderState { source } if source == expected));
    }
}

#[test]
fn unpublished_tip_output_motion_field_is_a_typed_reference_state_error() {
    let (sequence, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.inter.as_mut().expect("inter control").ref_frame_idx = [0].into_iter().collect();
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_motion_fields =
        RefSlots::from_iter_checked([Some(super::super::MotionFieldHandle::pending_with_layout(
            super::super::MotionFieldLayout::new(16, 16, 16).expect("valid motion-field layout"),
        ))])
        .expect("reference slots fit");
    let geometry = super::super::FrameDecodeGeometry::new(
        &core,
        &sequence,
        splot_recon::BitDepth::Eight,
        false,
    )
    .expect("valid frame geometry");

    let error = super::super::block::tip::reconstruct_output(
        &mut super::super::InterDecodeScratch::default(),
        splot_recon::FramePlaneSamples::default(),
        &sequence,
        &core,
        &reference,
        geometry,
        offset,
    )
    .expect_err("unpublished TIP-output motion field");
    assert!(matches!(
        error,
        DecodeError::ReferenceState {
            source: crate::error::DecodeReferenceStateError::MissingMotionFieldPublication,
        }
    ));
}

#[test]
fn impossible_sef_state_is_a_typed_header_state_error() {
    let (sequence, _) = fixture_sequence_and_key_core(SEF_FAMILIES_FIXTURE);
    let parsed = parse_ivf_fixture(SEF_FAMILIES_FIXTURE, "SEF families");
    let envelope = parsed
        .frames
        .iter()
        .flat_map(|frame| &frame.obus)
        .find(|envelope| envelope.header.obu_type.is_sef())
        .copied()
        .expect("SEF OBU");
    let num_ref_frames = usize::from(
        sequence
            .inter
            .as_ref()
            .expect("sequence inter config")
            .num_ref_frames,
    );
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_valid = RefSlots::from_iter_checked(core::iter::repeat_n(false, num_ref_frames))
        .expect("reference slots fit");
    reference.ref_order_hint = RefSlots::from_iter_checked(core::iter::repeat_n(0, num_ref_frames))
        .expect("reference slots fit");
    let mut core =
        super::super::parse_inter_frame_activation(envelope, &sequence, &reference, true, Some(2))
            .expect("complete SEF state");
    core.immediate_output_frame = None;

    let error = super::super::validate_sef_frame_core(&core, &reference, envelope.offset, Some(2))
        .expect_err("incomplete SEF state");
    assert!(matches!(
        error,
        DecodeError::HeaderState {
            source: DecodeHeaderStateError::IncompleteShowExistingFrame
        }
    ));
}

#[test]
fn ras_unlisted_long_term_reference_is_a_malformed_source_diagnostic() {
    let (_, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.obu_type = splot_core::types::ObuType::RasFrame;
    core.ref_long_term_ids = vec![3];
    let inter = core.inter.as_mut().expect("inter control");
    inter.num_total_refs = Some(1);
    inter.ref_frame_idx = [0].into_iter().collect();
    let mut reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");
    reference.ref_long_term_id =
        RefSlots::from_iter_checked([Some(5)]).expect("reference slots fit");

    let error = super::super::validate_ras_reference_ids(&core, &reference, offset, Some(1))
        .expect_err("unlisted RAS reference");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("6.17.2"));
    assert_eq!(issue.offset(), Some(offset));
    assert_eq!(issue.frame_index(), Some(1));
    assert_eq!(
        issue.message(),
        "RAS reference slot 0 has RefLongTermId 5, which is absent from the frame's listed \
         long-term IDs [3]"
    );
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("malformed RAS input must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);

    core.ref_long_term_ids = vec![5];
    super::super::validate_ras_reference_ids(&core, &reference, offset, Some(1))
        .expect("listed RAS reference");
}

#[test]
fn ras_out_of_range_reference_slot_is_a_malformed_source_diagnostic() {
    let (_, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.obu_type = splot_core::types::ObuType::RasFrame;
    let inter = core.inter.as_mut().expect("inter control");
    inter.num_total_refs = Some(1);
    inter.ref_frame_idx = [1].into_iter().collect();
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error = super::super::validate_ras_reference_ids(&core, &reference, offset, Some(1))
        .expect_err("out-of-range RAS reference slot");
    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("6.17.2"));
    assert_eq!(issue.offset(), Some(offset));
    assert_eq!(issue.frame_index(), Some(1));
    assert_eq!(
        issue.message(),
        "RAS reference slot 1 is outside the active reference map of 0 slots"
    );
    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("out-of-range RAS slot must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

#[test]
fn ras_slot_conformance_precedes_ccso_reference_reuse() {
    let (sequence, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.obu_type = splot_core::types::ObuType::RasFrame;
    let inter = core.inter.as_mut().expect("inter control");
    inter.num_total_refs = Some(1);
    inter.ref_frame_idx = [1].into_iter().collect();
    let ccso = core.ccso_params.as_mut().expect("CCSO state");
    ccso.planes
        .push(splot_core::headers::frame::CcsoPlaneParams {
            ccso_planes: true,
            reuse_ccso: true,
            sb_reuse_ccso: false,
            ccso_ref_idx: Some(0),
            ccso_bo_only: None,
            ccso_scale_idx: None,
            ccso_quant_idx: None,
            ccso_ext_filter: None,
            ccso_edge_clf: None,
            ccso_max_band_log2: None,
            ccso_offset_idx: splot_core::headers::frame::CcsoOffsets::default(),
        });
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error = super::super::validate_and_resolve_inter_frame_core(
        &mut core,
        &sequence,
        &reference,
        offset,
        Some(1),
    )
    .expect_err("RAS conformance must precede CCSO reuse");
    assert!(matches!(error, DecodeError::MalformedSource { .. }));
}

#[test]
fn parser_coverage_precedes_ras_slot_conformance() {
    let (sequence, mut core, offset) =
        parse_inter_core_for_validation(TWO_FRAME_INTER_FIXTURE).expect("inter core");
    core.obu_type = splot_core::types::ObuType::RasFrame;
    core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
        feature_id: "AV2-5.18.7-SEGMENTATION-TILING",
    };
    let inter = core.inter.as_mut().expect("inter control");
    inter.num_total_refs = Some(1);
    inter.ref_frame_idx = [1].into_iter().collect();
    let reference = super::super::InterReferenceState::<u8>::empty().expect("reference state");

    let error = super::super::validate_and_resolve_inter_frame_core(
        &mut core,
        &sequence,
        &reference,
        offset,
        Some(1),
    )
    .expect_err("parser coverage must precede RAS conformance");
    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("expected unsupported feature, got {error}");
    };
    assert_eq!(unsupported.reason(), "AV2-5.18.7-SEGMENTATION-TILING");
    assert_eq!(unsupported.spec_section(), "5.18.2");
}

#[test]
fn out_of_range_primary_reference_is_a_malformed_source_diagnostic() {
    let error = decode_inter_frame_after_core_mutation(TWO_FRAME_INTER_FIXTURE, |core| {
        let inter = core.inter.as_mut().unwrap();
        inter.signal_primary_ref_frame = Some(true);
        inter.primary_ref_frame = Some(6);
        inter.disable_cross_frame_cdf_init = Some(true);
        inter.ref_frame_idx = [0].into_iter().collect();
        inter.num_total_refs = Some(1);
    })
    .expect_err("primary reference must be inside the active map");

    let DecodeError::MalformedSource { issue } = &error else {
        panic!("expected malformed source, got {error}");
    };
    assert_eq!(
        issue.kind(),
        crate::DecodeSourceIssueKind::FrameHeaderConformanceError
    );
    assert_eq!(issue.spec_section(), Some("6.17"));
    assert!(issue.offset().is_some());
    assert_eq!(issue.frame_index(), Some(1));
    assert_eq!(
        issue.message(),
        "primary reference index 6 is outside the active 1-entry map"
    );

    let report = crate::DecodeDiagnosticReport::from_decode_error(&error)
        .expect("malformed frame-header input must remain user-reportable");
    assert_eq!(report.diagnostic.rule_id, crate::MALFORMED_SOURCE_RULE_ID);
}

fn repack_frame_obu(
    bytes: &[u8],
    target_frame_index: usize,
    mutate_payload: impl FnOnce(&[u8]) -> Vec<u8>,
) -> Vec<u8> {
    let parsed = parse_ivf_fixture(bytes, "bru inactive");
    let mut header = parsed.header.expect("fixture has an IVF header");
    header.frame_count = u32::try_from(parsed.frames.len()).expect("frame count fits u32");
    let mut out = Vec::new();
    write_repacked_ivf_header(&mut out, &header);
    let mut mutate = Some(mutate_payload);
    for (frame_index, frame) in parsed.frames.iter().enumerate() {
        let mut frame_writer = BitWriter::new();
        for envelope in &frame.obus {
            let replaced;
            let obu_payload = if frame_index == target_frame_index
                && envelope.header.obu_type == splot_core::types::ObuType::RegularTileGroup
            {
                replaced = mutate.take().expect("single target frame OBU")(envelope.payload);
                replaced.as_slice()
            } else {
                envelope.payload
            };
            write_annexb_obu(&mut frame_writer, &envelope.header, obu_payload)
                .expect("repack fixture OBU");
        }
        write_repacked_ivf_frame(&mut out, frame.frame.pts, &frame_writer.into_bytes());
    }
    out
}

fn inspect_bru_refresh_fixture(
    bytes: &[u8],
    expected_frames: usize,
    src_index: usize,
    inactive_index: usize,
    expect_distinct_slot: bool,
) {
    let options = DecodeOptions::default();
    let frames = crate::pipeline::collect_frames_from_plan(bytes, &options, 1, |frame| {
        Ok((
            frame.ccso_header.clone().expect("frame header"),
            frame.motion_field.field().cloned(),
            frame.segment_ids.product().cloned(),
            frame.ready_frame()?,
        ))
    })
    .expect("collect frames");
    assert_eq!(frames.len(), expected_frames);

    let (ref core_inactive, ref mf_inactive, ref seg_inactive, ref decoded_inactive) =
        frames[inactive_index];
    let inter_inactive = core_inactive
        .inter
        .as_ref()
        .expect("bru_inactive inter control");
    assert_eq!(inter_inactive.use_bru, Some(true));
    assert_eq!(inter_inactive.bru_inactive, Some(true));
    let bru_ref =
        usize::try_from(inter_inactive.bru_ref.expect("bru_ref")).expect("bru_ref fits usize");
    assert!(bru_ref > 0, "expected nonzero bru_ref, got {bru_ref}");
    let ref_slot = inter_inactive.ref_frame_idx[bru_ref];
    if expect_distinct_slot {
        assert_ne!(
            usize::try_from(ref_slot).expect("ref_slot"),
            bru_ref,
            "bru_ref {bru_ref} should map to a different physical DPB slot"
        );
    }
    assert_ne!(
        core_inactive.refresh_frame_flags.expect("refresh flags") & (1 << ref_slot),
        0,
        "bru_inactive frame must refresh its bru_ref slot {ref_slot}"
    );
    assert_eq!(core_inactive.disable_cdf_update, Some(true));
    assert!(core_inactive.ccso_params.is_none());
    assert!(core_inactive.lr_params.is_none());

    let mf_inactive = mf_inactive.as_ref().expect("bru_inactive motion field");
    assert_eq!(
        mf_inactive.stored_cell(0, 0),
        Some([bru_ref as u8, 0, 0, bru_ref as u8, 0, 0])
    );
    assert!(seg_inactive.is_some());

    let (_, _, _, ref decoded_src) = frames[src_index];
    match (decoded_src, decoded_inactive) {
        (PipelineDecodedFrame::Eight(src), PipelineDecodedFrame::Eight(inactive)) => {
            assert_eq!(&**inactive, &**src);
        }
        (PipelineDecodedFrame::Ten(src), PipelineDecodedFrame::Ten(inactive)) => {
            assert_eq!(&**inactive, &**src);
        }
        _ => panic!("mismatched frame bit depths"),
    }

    if expected_frames > inactive_index + 1 {
        let (ref core_next, _, _, ref decoded_next) = frames[inactive_index + 1];
        let inter_next = core_next.inter.as_ref().expect("next inter control");
        assert!(
            inter_next.ref_frame_idx.contains(&ref_slot),
            "frame {} must retain frame {inactive_index}'s refreshed slot {ref_slot} in {:?}",
            inactive_index + 1,
            inter_next.ref_frame_idx
        );
        match (decoded_inactive, decoded_next) {
            (PipelineDecodedFrame::Eight(inactive), PipelineDecodedFrame::Eight(next)) => {
                assert_ne!(&**next, &**inactive);
            }
            (PipelineDecodedFrame::Ten(inactive), PipelineDecodedFrame::Ten(next)) => {
                assert_ne!(&**next, &**inactive);
            }
            _ => panic!("mismatched frame bit depths"),
        }
    }
}

#[test]
fn bru_inactive_refresh_vectors_exercise_nonzero_bru_ref_slot_reuse() {
    inspect_bru_refresh_fixture(BRU_INACTIVE_FIXTURE, 5, 2, 4, false);
    inspect_bru_refresh_fixture(BRU_INACTIVE_REFRESH_8BIT_FIXTURE, 8, 3, 5, true);
    inspect_bru_refresh_fixture(BRU_INACTIVE_REFRESH_10BIT_FIXTURE, 8, 3, 5, true);
}

#[test]
fn bru_inactive_truncated_header_and_trailing_bits_are_malformed_source_and_recoverable() {
    let options = DecodeOptions::default();

    let truncated = repack_frame_obu(BRU_INACTIVE_REFRESH_8BIT_FIXTURE, 5, |payload| {
        payload[..payload.len() - 1].to_vec()
    });
    let Err(err) = decode_frames_from_plan(&truncated, &options) else {
        panic!("truncated bru_inactive header decoded successfully");
    };
    let DecodeError::MalformedSource { issue } = &err else {
        panic!("expected malformed source for truncated bru_inactive header, got {err}");
    };
    assert_eq!(issue.spec_section(), Some("6.2.1"));
    assert_eq!(issue.frame_index(), Some(5));

    for (flip_mask, label) in [(0x01u8, "nonzero zero-bit"), (0x08u8, "missing one-bit")] {
        let bad_tail = repack_frame_obu(BRU_INACTIVE_REFRESH_8BIT_FIXTURE, 5, |payload| {
            let mut copy = payload.to_vec();
            let last = copy.len() - 1;
            copy[last] ^= flip_mask;
            copy
        });
        let Err(err) = decode_frames_from_plan(&bad_tail, &options) else {
            panic!("malformed bru_inactive trailing_bits ({label}) decoded successfully");
        };
        let DecodeError::MalformedSource { issue } = &err else {
            panic!("expected malformed source ({label}), got {err}");
        };
        assert_eq!(issue.spec_section(), Some("5.19"));
        assert_eq!(issue.frame_index(), Some(5));

        for (threads, frame_delay) in [(1usize, 1usize), (2, 2), (4, 4)] {
            let settings = crate::DecoderSettings::default()
                .with_threads(crate::ThreadCount::from(threads))
                .with_frame_delay(crate::FrameDelay::from(frame_delay));
            let mut decoder = crate::Decoder::new(settings).unwrap();
            let mut reader = splot_core::stream_reader::TemporalUnitReader::new(&bad_tail[..]);
            let mut saw_malformed = false;
            let poll_bad = |decoder: &mut crate::Decoder| -> bool {
                loop {
                    match decoder.get_picture() {
                        Ok(Some(_)) => {}
                        Ok(None) => return false,
                        Err(DecodeError::MalformedSource { issue }) => {
                            assert_eq!(issue.spec_section(), Some("5.19"));
                            return true;
                        }
                        Err(other) => panic!("unexpected error ({label}): {other}"),
                    }
                }
            };
            'feed: while let Some(unit) = reader.next_unit().unwrap() {
                let splot_core::stream_reader::StreamUnit::IvfFrame { payload, .. } = unit else {
                    continue;
                };
                if payload.is_empty() {
                    continue;
                }
                loop {
                    match decoder.send_data(payload, crate::DataProps::default()) {
                        Ok(crate::SendStatus::Accepted) => {
                            if poll_bad(&mut decoder) {
                                saw_malformed = true;
                                break 'feed;
                            }
                            break;
                        }
                        Ok(crate::SendStatus::Full) => {
                            if poll_bad(&mut decoder) {
                                saw_malformed = true;
                                break 'feed;
                            }
                            std::thread::yield_now();
                        }
                        Err(DecodeError::MalformedSource { issue }) => {
                            assert_eq!(issue.spec_section(), Some("5.19"));
                            saw_malformed = true;
                            break 'feed;
                        }
                        Err(other) => panic!("unexpected send error ({label}): {other}"),
                    }
                }
            }
            if !saw_malformed {
                decoder.drain();
                saw_malformed = poll_bad(&mut decoder);
            }
            assert!(
                saw_malformed,
                "streaming decoder missed error ({label}, threads={threads}, delay={frame_delay})"
            );

            decoder.flush();
            let mut intact_reader = splot_core::stream_reader::TemporalUnitReader::new(
                BRU_INACTIVE_REFRESH_8BIT_FIXTURE,
            );
            let mut recovered = 0usize;
            while let Some(unit) = intact_reader.next_unit().unwrap() {
                let splot_core::stream_reader::StreamUnit::IvfFrame { payload, .. } = unit else {
                    continue;
                };
                if payload.is_empty() {
                    continue;
                }
                while decoder
                    .send_data(payload, crate::DataProps::default())
                    .unwrap()
                    == crate::SendStatus::Full
                {
                    while decoder.get_picture().unwrap().is_some() {
                        recovered += 1;
                    }
                    std::thread::yield_now();
                }
                while decoder.get_picture().unwrap().is_some() {
                    recovered += 1;
                }
            }
            decoder.drain();
            while decoder.get_picture().unwrap().is_some() {
                recovered += 1;
            }
            assert_eq!(
                recovered, 8,
                "recovery after flush failed ({label}, threads={threads}, delay={frame_delay})"
            );
        }
    }
}

#[test]
fn bru_inactive_zero_remaining_trailing_bits_and_incompatible_refs_fail_closed() {
    struct OrderHintObserver {
        order_hint_start: Option<u64>,
    }

    impl splot_core::syntax::SyntaxObserver for OrderHintObserver {
        fn enter_scope(&mut self, _: splot_core::syntax::ScopeKind) {}
        fn exit_scope(&mut self) {}
        fn on_field_read(&mut self, field: splot_core::syntax::SyntaxFieldRead) {
            if field.name == "order_hint_lsb" {
                self.order_hint_start = Some(field.span.start_bit);
            }
        }
        fn on_field_absent(&mut self, _: splot_core::syntax::SyntaxFieldAbsent) {}
        fn on_error(&mut self, _: splot_core::syntax::SyntaxErrorRef<'_>) {}
    }

    let parsed = parse_ivf_fixture(BRU_INACTIVE_REFRESH_8BIT_FIXTURE, "bru refresh");
    let frame5_obu = parsed.frames[5]
        .obus
        .iter()
        .find(|o| o.header.obu_type == splot_core::types::ObuType::RegularTileGroup)
        .copied()
        .unwrap();
    let frame5_candidate =
        plan_fixture(BRU_INACTIVE_REFRESH_8BIT_FIXTURE, &DecodeOptions::default())
            .frame_candidates_all()
            .nth(5)
            .expect("frame 5 candidate")
            .clone();
    let (mut sequence, _) = fixture_sequence_and_key_core(BRU_INACTIVE_REFRESH_8BIT_FIXTURE);
    let mut reference = super::super::InterReferenceState::<u8>::with_slot_count(8).unwrap();
    reference.ref_valid =
        RefSlots::from_iter_checked([true, true, true, true, false, false, false, false]).unwrap();
    reference.ref_order_hint = RefSlots::from_iter_checked([0, 2, 3, 4, 0, 0, 0, 0]).unwrap();
    reference.ref_order_hint_lsbs = RefSlots::from_iter_checked([0, 2, 3, 4, 0, 0, 0, 0]).unwrap();
    reference.ref_implicit_output_frame = RefSlots::from_iter_checked([false; 8]).unwrap();
    reference.ref_immediate_output_frame = RefSlots::from_iter_checked([true; 8]).unwrap();
    reference.ref_frame_width = RefSlots::from_iter_checked([128; 8]).unwrap();
    reference.ref_frame_height = RefSlots::from_iter_checked([64; 8]).unwrap();
    reference.ref_base_q_idx = RefSlots::from_iter_checked([220; 8]).unwrap();
    reference.ref_chroma_ac_deltas = RefSlots::from_iter_checked([[0, 0]; 8]).unwrap();
    reference.ref_num_total_refs = RefSlots::from_iter_checked([4; 8]).unwrap();
    reference.ref_counter = RefSlots::from_iter_checked([0, 2, 3, 4, 0, 0, 0, 0]).unwrap();
    reference.ref_is_inter =
        RefSlots::from_iter_checked([false, true, true, true, false, false, false, false]).unwrap();

    let intact_core = super::super::parse_inter_frame_core(
        frame5_obu,
        &sequence,
        &reference,
        true,
        None,
        Some(5),
    )
    .expect("intact frame 5 core");
    assert_eq!(
        intact_core.status,
        FrameHeaderParseStatus::InterHeaderComplete,
        "unexpected intact core: {intact_core:#?}"
    );
    assert_eq!(
        intact_core.inter.as_ref().and_then(|i| i.bru_inactive),
        Some(true)
    );

    let mut obs_reader = BitReader::new(frame5_obu.payload, ByteOffset::new(0));
    assert_eq!(obs_reader.read_bit().unwrap(), 1);
    let mut observer = OrderHintObserver {
        order_hint_start: None,
    };
    let obs_input = splot_core::headers::frame::FrameHeaderParseInput {
        obu_type: frame5_obu.header.obu_type,
        first_picture_in_tu: true,
        active_sequence: Some(&sequence),
        mfh_record: None,
        reference_state: reference.header_view(),
    };
    splot_core::headers::frame::parse_frame_header_core_observed(
        &mut obs_reader,
        &obs_input,
        &mut observer,
    )
    .expect("observed frame 5 header");
    let order_hint_payload_bit = 1 + observer.order_hint_start.expect("order_hint_lsb observed");

    let header_plus_first_tg = 1 + intact_core.consumed_bits;
    let extra_order_hint_bits = u8::try_from((8 - (header_plus_first_tg % 8)) % 8).unwrap();
    sequence.inter.as_mut().unwrap().order_hint_bits += extra_order_hint_bits;

    let mut reader = BitReader::new(frame5_obu.payload, ByteOffset::new(0));
    let mut zero_tail_writer = BitWriter::new();
    for _ in 0..order_hint_payload_bit {
        zero_tail_writer
            .write_bit(reader.read_bit().unwrap())
            .unwrap();
    }
    zero_tail_writer
        .write_bits(0, u32::from(extra_order_hint_bits))
        .unwrap();
    for _ in order_hint_payload_bit..header_plus_first_tg {
        zero_tail_writer
            .write_bit(reader.read_bit().unwrap())
            .unwrap();
    }
    let zero_tail_payload = zero_tail_writer.into_bytes();
    assert_eq!(
        (header_plus_first_tg + u64::from(extra_order_hint_bits)) % 8,
        0
    );
    let mut zero_tail_obu = frame5_obu;
    zero_tail_obu.payload = &zero_tail_payload;
    zero_tail_obu.size = u32::from(zero_tail_obu.header.header_size_bytes)
        + u32::try_from(zero_tail_payload.len()).unwrap();
    let err = super::super::parse_inter_frame_core(
        zero_tail_obu,
        &sequence,
        &reference,
        true,
        None,
        Some(5),
    )
    .unwrap_err();
    let DecodeError::MalformedSource { issue } = &err else {
        panic!("expected malformed source on 0 remaining trailing bits, got {err}");
    };
    assert_eq!(issue.spec_section(), Some("5.19"));

    let bru_slot = super::super::bru_ref_slot(&intact_core).unwrap() as usize;
    assert_eq!(bru_slot, 1);
    super::super::validate_and_resolve_inter_frame_core(
        &mut intact_core.clone(),
        &sequence,
        &reference,
        frame5_obu.offset,
        Some(5),
    )
    .expect("intact bru_inactive core validates");

    reference.ref_frame_width[bru_slot] = 64;
    let err = super::super::validate_and_resolve_inter_frame_core(
        &mut intact_core.clone(),
        &sequence,
        &reference,
        frame5_obu.offset,
        Some(5),
    )
    .unwrap_err();
    reference.ref_frame_width[bru_slot] = 128;
    let DecodeError::MalformedSource { issue } = &err else {
        panic!("expected malformed source on mismatched bru_ref size, got {err}");
    };
    assert_eq!(issue.spec_section(), Some("6.17.2"));

    let mut non_imm_core = intact_core.clone();
    non_imm_core.immediate_output_frame = Some(false);
    let err = super::super::validate_and_resolve_inter_frame_core(
        &mut non_imm_core,
        &sequence,
        &reference,
        frame5_obu.offset,
        Some(5),
    )
    .unwrap_err();
    assert!(matches!(err, DecodeError::MalformedSource { .. }));

    let mut no_refresh_core = intact_core.clone();
    no_refresh_core.refresh_frame_flags = Some(0);
    let err = super::super::validate_and_resolve_inter_frame_core(
        &mut no_refresh_core,
        &sequence,
        &reference,
        frame5_obu.offset,
        Some(5),
    )
    .unwrap_err();
    assert!(matches!(err, DecodeError::MalformedSource { .. }));

    let mut products = super::super::FrameProductWriters::fresh().unwrap();
    let Err(err) = super::super::decode_bridge_frame(
        &frame5_candidate,
        frame5_obu,
        std::sync::Arc::new(intact_core),
        &sequence,
        &DecodeOptions::default(),
        &reference,
        BitDepth::Eight,
        &mut products,
    ) else {
        panic!("missing reference slot decoded successfully");
    };
    assert!(matches!(
        err,
        DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::MissingFrame { slot }
        } if slot == bru_slot
    ));
}

#[test]
fn bru_lr_vectors_exercise_partial_bru_shifted_stripes_and_reference_consumers() {
    use crate::bitstream::tile_payload::{BRU_ACTIVE, LrUnitRestorationType};

    let fixtures: [(&str, &[u8], LrUnitRestorationType, Option<bool>); 6] = [
        (
            "syn-bru-lr-stripe-wienerns-8bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-lr-stripe-wienerns-8bit.ivf"
            ),
            LrUnitRestorationType::WienerNonsep,
            None,
        ),
        (
            "syn-bru-lr-stripe-wienerns-10bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-lr-stripe-wienerns-10bit.ivf"
            ),
            LrUnitRestorationType::WienerNonsep,
            None,
        ),
        (
            "syn-bru-lr-stripe-pcwiener-8bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-lr-stripe-pcwiener-8bit.ivf"
            ),
            LrUnitRestorationType::PcWiener,
            None,
        ),
        (
            "syn-bru-lr-stripe-pcwiener-10bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-lr-stripe-pcwiener-10bit.ivf"
            ),
            LrUnitRestorationType::PcWiener,
            None,
        ),
        (
            "syn-bru-lr-multitile-cross-8bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-lr-multitile-cross-8bit.ivf"
            ),
            LrUnitRestorationType::PcWiener,
            Some(false),
        ),
        (
            "syn-bru-lr-multitile-isolated-8bit",
            include_bytes!(
                "../../../../../../tests/conformance/vectors/valid/syn-bru-lr-multitile-isolated-8bit.ivf"
            ),
            LrUnitRestorationType::PcWiener,
            Some(true),
        ),
    ];

    for (name, bytes, expected_unit_type, expected_isolated_tiles) in fixtures {
        let (headers, audits) =
            crate::pipeline::collect_bru_lr_audit(bytes, &DecodeOptions::default())
                .expect("audit decode");
        assert_eq!(headers.len(), 6, "{name}: expected 6 frames");
        assert_eq!(audits.len(), 6, "{name}: expected 6 filter audits");

        let mut bru_lr_indices = Vec::new();
        let mut saw_shifted_crossing = false;
        let mut saw_trimmed_span = false;
        for (idx, audit) in audits.iter().enumerate() {
            if let Some(isolated) = expected_isolated_tiles {
                assert_eq!(
                    audit.disable_loopfilters_across_tiles, isolated,
                    "{name} f{idx}: tile-isolation flag mismatch"
                );
            }
            if !audit.use_bru {
                continue;
            }
            assert!(
                !audit.bru_inactive,
                "{name} f{idx}: expected active BRU frame"
            );
            let active_sbs = audit
                .superblock_modes
                .iter()
                .filter(|(_, _, mode)| *mode == Some(BRU_ACTIVE))
                .count();
            let inactive_sbs = audit
                .superblock_modes
                .iter()
                .filter(|(_, _, mode)| mode.is_some() && *mode != Some(BRU_ACTIVE))
                .count();
            assert!(
                active_sbs > 0 && inactive_sbs > 0,
                "{name} f{idx}: expected partial BRU"
            );
            if !audit.lr_uses_lr {
                continue;
            }
            let expected_frame_rtype = match expected_unit_type {
                LrUnitRestorationType::WienerNonsep => {
                    splot_core::headers::frame::FrameRestorationType::WienerNonsep
                }
                LrUnitRestorationType::PcWiener => {
                    splot_core::headers::frame::FrameRestorationType::PcWiener
                }
                LrUnitRestorationType::None => {
                    splot_core::headers::frame::FrameRestorationType::None
                }
            };
            assert!(
                audit
                    .plane_restoration_types
                    .contains(&expected_frame_rtype)
                    || audit
                        .plane_restoration_types
                        .contains(&splot_core::headers::frame::FrameRestorationType::Switchable),
                "{name} f{idx}: expected plane restoration {expected_frame_rtype:?}, got {:?}",
                audit.plane_restoration_types
            );
            let has_target_unit = audit
                .pre_bru_blocks
                .iter()
                .any(|b| b.restoration_type == expected_unit_type);
            if !has_target_unit {
                continue;
            }
            bru_lr_indices.push(idx);
            if audit.post_bru_blocks.iter().any(|b| {
                b.restoration_type == expected_unit_type
                    && ((b.y << usize::from(b.plane != 0))
                        ..((b.y + b.height) << usize::from(b.plane != 0)))
                        .contains(&56)
            }) {
                saw_shifted_crossing = true;
            }
            if audit.post_bru_blocks.len() < audit.pre_bru_blocks.len()
                || audit
                    .post_bru_blocks
                    .iter()
                    .map(|b| b.width * b.height)
                    .sum::<usize>()
                    < audit
                        .pre_bru_blocks
                        .iter()
                        .map(|b| b.width * b.height)
                        .sum::<usize>()
            {
                saw_trimmed_span = true;
            }
        }
        assert!(
            !bru_lr_indices.is_empty(),
            "{name}: missing frame combining use_bru == 1 and active {expected_unit_type:?}"
        );
        assert!(
            saw_shifted_crossing || saw_trimmed_span,
            "{name}: expected shifted-stripe crossing or trimmed inactive LR span"
        );
        let first_idx = bru_lr_indices[0];
        assert!(
            first_idx + 1 < headers.len(),
            "{name}: missing post-BRU consumer frame"
        );
        let refreshed_mask = headers[first_idx].refresh_frame_flags.unwrap_or(0);
        assert_ne!(
            refreshed_mask, 0,
            "{name} f{first_idx}: expected reference refresh"
        );
        let consumer_refs = &headers[first_idx + 1]
            .inter
            .as_ref()
            .expect("consumer inter control")
            .ref_frame_idx;
        assert!(
            consumer_refs
                .iter()
                .any(|&slot| (refreshed_mask & (1u32 << slot)) != 0),
            "{name} f{}: expected ref consumer to retain refreshed slot from f{first_idx}",
            first_idx + 1
        );
    }

    let cross_frames =
        crate::pipeline::decode_frames_from_plan(fixtures[4].1, &DecodeOptions::default())
            .expect("cross decode");
    let isolated_frames =
        crate::pipeline::decode_frames_from_plan(fixtures[5].1, &DecodeOptions::default())
            .expect("isolated decode");
    let cross_samples = match &cross_frames[3] {
        crate::pipeline::PipelineDecodedFrame::Eight(f) => &**f,
        crate::pipeline::PipelineDecodedFrame::Ten(_) => panic!("expected 8-bit frame"),
    };
    let isolated_samples = match &isolated_frames[3] {
        crate::pipeline::PipelineDecodedFrame::Eight(f) => &**f,
        crate::pipeline::PipelineDecodedFrame::Ten(_) => panic!("expected 8-bit frame"),
    };
    assert_ne!(
        cross_samples, isolated_samples,
        "multitile cross and isolated BRU+LR outputs must diverge across tile borders"
    );
}

#[test]
fn bru_lr_corrupt_and_truncated_tile_payloads_fail_closed() {
    for fixture in [
        include_bytes!(
            "../../../../../../tests/conformance/vectors/valid/syn-bru-lr-stripe-wienerns-8bit.ivf"
        )
        .as_slice(),
        include_bytes!(
            "../../../../../../tests/conformance/vectors/valid/syn-bru-lr-stripe-pcwiener-8bit.ivf"
        )
        .as_slice(),
    ] {
        let truncated =
            repack_frame_obu(fixture, 3, |payload| payload[..payload.len() / 2].to_vec());
        let err = crate::pipeline::decode_frames_from_plan(&truncated, &DecodeOptions::default())
            .err()
            .expect("truncated BRU+LR tile payload must fail");
        assert!(
            matches!(err, DecodeError::MalformedSource { .. }),
            "expected MalformedSource for truncated BRU+LR tile payload, got {err}"
        );

        let flipped = repack_frame_obu(fixture, 3, |payload| {
            let mut copy = payload.to_vec();
            let last = copy.len() - 1;
            copy[last] ^= 0xFF;
            if last > 2 {
                copy[last - 2] ^= 0x5A;
            }
            copy
        });
        assert!(
            crate::pipeline::decode_frames_from_plan(&flipped, &DecodeOptions::default()).is_err(),
            "corrupted BRU+LR tile payload must fail closed without panicking"
        );
    }
}
