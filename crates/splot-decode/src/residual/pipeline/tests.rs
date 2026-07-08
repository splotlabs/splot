// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::tile::block_context::{BlockRect, ChromaSampling, TxShape};
use splot_recon::{BitDepth, DpcmDirection};

impl GeneralIntraResidualPlan {
    fn plane_plan(&self, plane_id: PlaneId) -> Option<ResidualPlanePlan> {
        self.planes
            .iter()
            .find(|plane| plane.plane_id == plane_id)
            .copied()
    }
}

#[derive(Clone, Copy)]
struct Case {
    label: &'static str,
    rect: BlockRect,
    bit_depth: BitDepth,
    plane: PlaneId,
    expected_tx_log2: (u32, u32),
    expect_chroma: bool,
}

#[test]
fn plans_square_and_rectangular_residual_planes() {
    let cases = [
        Case {
            label: "square luma 8-bit",
            rect: BlockRect::new(0, 0, 16, 16),
            bit_depth: BitDepth::Eight,
            plane: PlaneId::Y,
            expected_tx_log2: (6, 6),
            expect_chroma: true,
        },
        Case {
            label: "square chroma-u 10-bit",
            rect: BlockRect::new(0, 0, 16, 16),
            bit_depth: BitDepth::Ten,
            plane: PlaneId::U,
            expected_tx_log2: (5, 5),
            expect_chroma: true,
        },
        Case {
            label: "square chroma-v dependency",
            rect: BlockRect::new(0, 0, 16, 16),
            bit_depth: BitDepth::Eight,
            plane: PlaneId::V,
            expected_tx_log2: (5, 5),
            expect_chroma: true,
        },
        Case {
            label: "rect luma",
            rect: BlockRect::new(0, 0, 16, 8),
            bit_depth: BitDepth::Eight,
            plane: PlaneId::Y,
            expected_tx_log2: (6, 5),
            expect_chroma: true,
        },
        Case {
            label: "rect chroma-u",
            rect: BlockRect::new(0, 0, 16, 8),
            bit_depth: BitDepth::Ten,
            plane: PlaneId::U,
            expected_tx_log2: (5, 4),
            expect_chroma: true,
        },
        Case {
            label: "rect chroma-v dependency",
            rect: BlockRect::new(0, 0, 16, 8),
            bit_depth: BitDepth::Eight,
            plane: PlaneId::V,
            expected_tx_log2: (5, 4),
            expect_chroma: true,
        },
    ];

    for case in cases {
        assert_case(case);
    }
}

#[test]
fn omits_chroma_plans_for_luma_only_blocks() {
    let block = BlockRect::new(0, 0, 16, 8);
    let ctx = ctx(block, BitDepth::Eight);
    let plan =
        GeneralIntraResidualPlan::rect(ctx, RectLumaPlan::Dc { use_tcq: true }, None, false, false)
            .expect("rect luma plan");
    assert!(plan.plane_plan(PlaneId::U).is_none());
    assert!(plan.plane_plan(PlaneId::V).is_none());
    assert_eq!(plan.transforms().chroma_tx(), None);
}

#[test]
fn chroma_dc_uses_generic_rect_reconstruction() {
    let block = BlockRect::new(0, 0, 16, 16);
    let ctx = ctx(block, BitDepth::Ten);
    let plan = GeneralIntraResidualPlan::square(
        ctx,
        IntraLumaPlan::Dc,
        Some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
        false,
        false,
        false,
    )
    .expect("square plan");

    for plane_id in [PlaneId::U, PlaneId::V] {
        assert_eq!(
            plan.plane_plan(plane_id)
                .expect("chroma plane")
                .reconstruction,
            ResidualReconstructionPlan::Rect { use_tcq: false }
        );
    }
}

#[test]
fn chroma_dpcm_direction_is_preserved_for_both_planes() {
    let block = BlockRect::new(0, 0, 16, 16);
    let ctx = ctx(block, BitDepth::Eight);
    let plan = GeneralIntraResidualPlan::square(
        ctx,
        IntraLumaPlan::Dc,
        Some(RectChromaPlan::Mode(
            SupportedChromaMode::Vertical,
            Some(DpcmDirection::Vertical),
        )),
        false,
        false,
        false,
    )
    .expect("square plan");

    for plane_id in [PlaneId::U, PlaneId::V] {
        assert_eq!(
            plan.plane_plan(plane_id)
                .expect("chroma plane")
                .reconstruction,
            ResidualReconstructionPlan::Chroma {
                mode: SupportedChromaMode::Vertical,
                dpcm: Some(DpcmDirection::Vertical)
            }
        );
    }
}

#[test]
fn fsc_coefficients_are_luma_only() {
    let block = BlockRect::new(0, 0, 16, 16);
    let ctx = ctx(block, BitDepth::Ten);
    let plan = GeneralIntraResidualPlan::square(
        ctx,
        IntraLumaPlan::Dc,
        Some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
        true,
        true,
        false,
    )
    .expect("square fsc plan");

    assert!(plan.plane_plan(PlaneId::Y).expect("luma").fsc_mode);
    assert!(!plan.plane_plan(PlaneId::U).expect("chroma u").fsc_mode);
    assert!(!plan.plane_plan(PlaneId::V).expect("chroma v").fsc_mode);
    assert!(
        plan.plane_plan(PlaneId::U)
            .expect("chroma u")
            .txb_skip_fsc_mode
    );
    assert!(
        plan.plane_plan(PlaneId::V)
            .expect("chroma v")
            .txb_skip_fsc_mode
    );
}

#[test]
fn large_luma_chunks_do_not_fill_parent_residual_block() {
    let block = BlockRect::new(0, 0, 32, 16);
    let ctx = ctx(block, BitDepth::Ten);
    let plan =
        GeneralIntraResidualPlan::rect(ctx, RectLumaPlan::Dc { use_tcq: true }, None, false, false)
            .expect("rect luma plan");
    let luma: Vec<_> = plan
        .planes
        .iter()
        .filter(|plane| plane.plane_id == PlaneId::Y)
        .copied()
        .collect();

    assert_eq!(luma.len(), 2);
    assert!(
        luma.iter()
            .all(|plane| (plane.tx.width4(), plane.tx.height4()) == (16, 16))
    );
    assert!(luma.iter().all(|plane| !plane.tx_fills_residual_block()));
    assert!(
        luma.iter()
            .all(|plane| (plane.residual_width4, plane.residual_height4) == (32, 16))
    );
}

#[test]
fn cfl_chroma_keeps_read_order_and_defers_reconstruction() {
    let block = BlockRect::new(0, 0, 32, 16);
    let ctx = ctx(block, BitDepth::Ten);
    let plan = GeneralIntraResidualPlan::rect(
        ctx,
        RectLumaPlan::Dc { use_tcq: true },
        Some(RectChromaPlan::Cfl {
            params: CflParams {
                index: crate::bitstream::tile_payload::CflIndex::DerivedAlpha,
                alpha_u: 0,
                alpha_v: 0,
                mh_dir: None,
            },
            cfl_ds_filter_index: 0,
            sb_mib: 16,
        }),
        false,
        false,
    )
    .expect("cfl rect plan");

    assert_large_chroma_order(&plan, true);
}

#[test]
fn non_cfl_chroma_keeps_chunk_interleaving() {
    let block = BlockRect::new(0, 0, 32, 16);
    let ctx = ctx(block, BitDepth::Ten);
    let plan = GeneralIntraResidualPlan::rect(
        ctx,
        RectLumaPlan::Dc { use_tcq: true },
        Some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
        false,
        false,
    )
    .expect("dc rect plan");

    assert_large_chroma_order(&plan, false);
}

#[test]
fn non_lossless_chroma_uses_partition_chroma_ref() {
    let block = BlockRect::new(1, 11, 1, 1);
    let chroma_ref = BlockRect::new(0, 10, 2, 2);
    let chroma_tx = TxShape::from_luma_4x4(2, 2).expect("valid chroma reference transform");
    let ctx = ctx(block, BitDepth::Eight).with_chroma_ref(chroma_ref, chroma_tx);
    let plan = GeneralIntraResidualPlan::rect(
        ctx,
        RectLumaPlan::Dc { use_tcq: false },
        Some(RectChromaPlan::Mode(SupportedChromaMode::Paeth, None)),
        false,
        false,
    )
    .expect("non-lossless chroma ref plan");
    let u = plan.plane_plan(PlaneId::U).expect("chroma u");

    assert_eq!((u.x, u.y), (20, 0));
    assert_eq!((u.tx.width_log2(), u.tx.height_log2()), (2, 2));
}

#[test]
fn non_lossless_yuv444_chroma_follows_each_residual_chunk() {
    let block = BlockRect::new(0, 0, 32, 16);
    let ctx = ctx_with_chroma(block, BitDepth::Eight, ChromaSampling::Yuv444);
    let plan = GeneralIntraResidualPlan::rect(
        ctx,
        RectLumaPlan::Dc { use_tcq: true },
        Some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
        false,
        false,
    )
    .expect("non-lossless yuv444 rect plan");

    let order: Vec<_> = plan
        .planes
        .iter()
        .map(|plane| {
            (
                plane.plane_id,
                plane.x,
                plane.y,
                plane.tx.width4(),
                plane.tx.height4(),
            )
        })
        .collect();
    assert_eq!(
        order,
        [
            (PlaneId::Y, 0, 0, 16, 16),
            (PlaneId::U, 0, 0, 16, 16),
            (PlaneId::V, 0, 0, 16, 16),
            (PlaneId::Y, 64, 0, 16, 16),
            (PlaneId::U, 64, 0, 16, 16),
            (PlaneId::V, 64, 0, 16, 16),
        ]
    );
}

#[test]
fn lossless_large_chroma_follows_each_residual_chunk() {
    let block = BlockRect::new(0, 0, 32, 16);
    let ctx = ctx(block, BitDepth::Eight);
    let plan = GeneralIntraResidualPlan::rect(
        ctx,
        RectLumaPlan::Dc { use_tcq: false },
        Some(RectChromaPlan::Mode(SupportedChromaMode::D45, None)),
        false,
        true,
    )
    .expect("lossless rect plan");

    let order: Vec<_> = plan
        .planes
        .iter()
        .map(|plane| (plane.plane_id, plane.x, plane.y))
        .collect();
    assert_eq!(
        order,
        [
            (PlaneId::Y, 0, 0),
            (PlaneId::U, 0, 0),
            (PlaneId::V, 0, 0),
            (PlaneId::Y, 64, 0),
            (PlaneId::U, 32, 0),
            (PlaneId::V, 32, 0),
        ]
    );
}

#[test]
fn lossless_v_handoff_uses_final_u_unit_flag() {
    let mut coeffs = empty_luma_coeffs();
    coeffs.all_zero = false;

    let final_u_zero = ResidualPlaneExecution {
        coeffs: coeffs.clone(),
        last_unit_nonzero: Some(false),
    };
    assert!(
        !final_u_zero
            .last_unit_nonzero
            .unwrap_or(!final_u_zero.coeffs.all_zero)
    );

    let final_u_nonzero = ResidualPlaneExecution {
        coeffs: coeffs.clone(),
        last_unit_nonzero: Some(true),
    };
    assert!(
        final_u_nonzero
            .last_unit_nonzero
            .unwrap_or(!final_u_nonzero.coeffs.all_zero)
    );

    let whole_u_nonzero = ResidualPlaneExecution {
        coeffs,
        last_unit_nonzero: None,
    };
    assert!(
        whole_u_nonzero
            .last_unit_nonzero
            .unwrap_or(!whole_u_nonzero.coeffs.all_zero)
    );
}

fn assert_large_chroma_order(plan: &GeneralIntraResidualPlan, defer: bool) {
    let order: Vec<_> = plan
        .planes
        .iter()
        .map(|plane| (plane.plane_id, plane.x, plane.y))
        .collect();
    assert_eq!(
        order,
        [
            (PlaneId::Y, 0, 0),
            (PlaneId::U, 0, 0),
            (PlaneId::V, 0, 0),
            (PlaneId::Y, 64, 0),
        ]
    );
    assert_eq!(
        plan.planes
            .iter()
            .filter(|plane| plane.plane_id != PlaneId::Y)
            .map(|plane| plane.defer_reconstruction)
            .collect::<Vec<_>>(),
        [defer, defer]
    );
}

#[test]
fn chroma_angle_delta_tracks_directional_follow_mode() {
    let luma = LumaTransformTypeContext::new(
        crate::bitstream::tile_payload::IntraYMode::D135_PRED_FOR_TEST,
        -3,
    );

    assert_eq!(
        chroma_angle_delta_uv(
            PlaneId::U,
            crate::bitstream::tile_payload::IntraYMode::D135_PRED_FOR_TEST.value(),
            luma,
        ),
        -3
    );
    assert_eq!(
        chroma_angle_delta_uv(
            PlaneId::V,
            crate::bitstream::tile_payload::IntraYMode::DC_PRED.value(),
            luma
        ),
        0
    );
    assert_eq!(
        chroma_angle_delta_uv(
            PlaneId::Y,
            crate::bitstream::tile_payload::IntraYMode::D135_PRED_FOR_TEST.value(),
            luma,
        ),
        0
    );
}

#[test]
fn palette_map_is_sliced_for_each_partitioned_transform_size() {
    let block = BlockRect::new(2, 3, 16, 16);
    let ctx = ctx(block, BitDepth::Ten);
    let palette = LumaPalette::new(4, [16, 64, 128, 240, 0, 0, 0, 0]).expect("palette");
    let plan = GeneralIntraResidualPlan::rect(
        ctx,
        RectLumaPlan::Palette {
            palette,
            use_tcq: false,
        },
        None,
        false,
        false,
    )
    .expect("palette rect plan");
    let plane = plan.plane_plan(PlaneId::Y).expect("luma plane");
    let parent_width = 1usize << plane.tx.width_log2();
    let parent_height = 1usize << plane.tx.height_log2();
    let parent_map: Vec<u8> = (0..parent_width * parent_height)
        .map(|idx| (idx % 251) as u8)
        .collect();

    for tx_size in 0..TX_WIDTH_LOG2.len() {
        let (log2_width, log2_height) = tx_size_log2(tx_size).expect("test tx size");
        let width = 1usize << log2_width;
        let height = 1usize << log2_height;
        let local_x = if width == parent_width { 0 } else { 4 };
        let local_y = if height == parent_height { 0 } else { 8 };
        let coeff = PositionedLumaCoeffBlock {
            x: plane.x + local_x,
            y: plane.y + local_y,
            tx_size,
            middle: false,
            coeffs: empty_luma_coeffs(),
        };
        let unit = plane
            .transform_unit_plan(&coeff)
            .unwrap_or_else(|error| panic!("tx_size {tx_size}: {error}"));
        assert_eq!(
            unit.reconstruction, plane.reconstruction,
            "tx_size {tx_size}"
        );

        let unit_map = plane
            .palette_color_map_for_unit(Some(&parent_map), &coeff)
            .unwrap_or_else(|error| panic!("tx_size {tx_size}: {error}"))
            .expect("palette unit map");
        let expected: Vec<u8> = (0..height)
            .flat_map(|row| {
                let start = (local_y + row) * parent_width + local_x;
                parent_map[start..start + width].iter().copied()
            })
            .collect();
        assert_eq!(unit_map, expected, "tx_size {tx_size}");
    }
}

#[test]
fn directional_first_d135_partition_handoff_stays_lossless_only() {
    let block_ctx = ctx(BlockRect::new(0, 0, 16, 16), BitDepth::Eight);
    let plan = GeneralIntraResidualPlan::square(
        block_ctx,
        IntraLumaPlan::DirectionalFirst {
            mode: SupportedDirectionalLumaMode::D135,
        },
        None,
        false,
        false,
        false,
    )
    .expect("d135 square plan");
    let plane = plan.plane_plan(PlaneId::Y).expect("luma plane");
    let non_origin = PositionedLumaCoeffBlock {
        x: 4,
        y: 0,
        tx_size: TX_4X4,
        middle: false,
        coeffs: empty_luma_coeffs(),
    };

    let error = plane
        .transform_unit_plan(&non_origin)
        .expect_err("non-lossless D135 partitioned units must fail closed");
    assert!(matches!(
        error,
        GeneralIntraResidualError::UnsupportedTransformPartition {
            reason: "general_intra_partitioned_interior_edge_prediction"
        }
    ));

    let mut lossless_coeffs = empty_luma_coeffs();
    lossless_coeffs.lossless = true;
    let lossless_unit = PositionedLumaCoeffBlock {
        coeffs: lossless_coeffs,
        ..non_origin
    };
    assert_eq!(
        plane
            .transform_unit_plan(&lossless_unit)
            .expect("lossless D135 partitioned unit"),
        ResidualPlanePlan {
            x: 4,
            y: 0,
            tx_size: TX_4X4,
            tx: TxShape::from_luma_4x4(1, 1).expect("4x4 tx shape"),
            residual_width4: 1,
            residual_height4: 1,
            zero_corners: false,
            reconstruction: ResidualReconstructionPlan::LumaRectMiddle {
                p_angle: crate::prediction::intra::directional_mode_p_angle(
                    SupportedDirectionalLumaMode::D135,
                ),
                use_tcq: false,
            },
            block_ctx: ctx(BlockRect::new(0, 1, 1, 1), BitDepth::Eight),
            ..plane
        }
    );
}

fn assert_case(case: Case) {
    let ctx = ctx(case.rect, case.bit_depth);
    let plan = if case.rect.width4() == case.rect.height4() {
        GeneralIntraResidualPlan::square(
            ctx,
            IntraLumaPlan::Dc,
            Some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
            true,
            false,
            false,
        )
    } else {
        GeneralIntraResidualPlan::rect(
            ctx,
            RectLumaPlan::Dc { use_tcq: true },
            case.expect_chroma
                .then_some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
            false,
            false,
        )
    }
    .unwrap_or_else(|error| panic!("{}: {}", case.label, error.reason_id()));
    let plane = plan
        .plane_plan(case.plane)
        .unwrap_or_else(|| panic!("{}: missing plane", case.label));
    assert_eq!(
        plane.tx.width_log2(),
        case.expected_tx_log2.0,
        "{}",
        case.label
    );
    assert_eq!(
        plane.tx.height_log2(),
        case.expected_tx_log2.1,
        "{}",
        case.label
    );
    assert_eq!(plane.coeff_plane, coeff_plane(case.plane), "{}", case.label);
}

fn ctx(block: BlockRect, bit_depth: BitDepth) -> BlockCtx {
    ctx_with_chroma(block, bit_depth, ChromaSampling::Yuv420)
}

fn ctx_with_chroma(block: BlockRect, bit_depth: BitDepth, chroma: ChromaSampling) -> BlockCtx {
    let tx = TxShape::from_luma_4x4(block.width4(), block.height4()).expect("test tx shape");
    BlockCtx::new(block, tx, 32, 32, bit_depth, chroma)
}

fn empty_luma_coeffs() -> crate::bitstream::tile_payload::LumaCoeffBlock {
    crate::bitstream::tile_payload::LumaCoeffBlock {
        all_zero: true,
        eob: 0,
        quant: Vec::new(),
        intra_ist: None,
        plane_tx_type: 0,
        use_tcq: false,
        lossless: false,
    }
}
