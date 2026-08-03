// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::bitstream::tile_payload::TileBlockDecodedState;
use crate::tile::block_context::{BlockRect, ChromaSampling, TxShape};
use splot_core::tables::conversion::{NUM_4X4_BLOCKS_HIGH, NUM_4X4_BLOCKS_WIDE, TX_HEIGHT_LOG2};
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
    let plan = GeneralIntraResidualPlan::rect(
        ctx,
        RectLumaPlan::Dc { use_tcq: true },
        None,
        false,
        None,
        false,
    )
    .expect("rect luma plan");
    assert!(plan.plane_plan(PlaneId::U).is_none());
    assert!(plan.plane_plan(PlaneId::V).is_none());
    assert_eq!(plan.chroma_tx(), None);
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
        None,
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
fn deblock_recorder_keeps_chroma_transform_unit_boundaries() {
    let tx_8x8 = rect_tx_size_from_log2(3, 3).expect("8x8 transform");
    let mut blocks = Vec::new();
    let mut chroma_blocks = [Vec::new(), Vec::new()];
    let mut tx_skip_records = Vec::new();
    let mut recorder = DeblockRecorder {
        blocks: &mut blocks,
        chroma_blocks: &mut chroma_blocks,
        tx_skip_records: &mut tx_skip_records,
        block_r: 0,
        block_c: 0,
        chroma_tx: Some(tx_8x8),
        chroma_subsampling: (1, 0),
        qindex: 37,
        lossless: true,
    };

    recorder.record_chroma_unit(PlaneId::U, 0, 8, tx_8x8);
    recorder.record_chroma_unit(PlaneId::U, 8, 8, tx_8x8);
    recorder.record_chroma_unit(PlaneId::V, 0, 8, tx_8x8);

    assert_eq!(chroma_blocks[0].len(), 2);
    assert_eq!(chroma_blocks[0][0].chroma_base_c, 0);
    assert_eq!(chroma_blocks[0][1].chroma_base_c, 4);
    assert_eq!(chroma_blocks[0][1].n4w, 4);
    assert_eq!(chroma_blocks[0][1].chroma_base_r, 2);
    assert_eq!(chroma_blocks[0][1].n4h, 2);
    assert_eq!(chroma_blocks[1].len(), 1);
    assert_eq!(chroma_blocks[1][0].qindex, 37);
    assert!(chroma_blocks[1][0].lossless);
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
        None,
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
        None,
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
fn chroma_part_applies_lossless_luma_fsc_only_to_reconstruction() {
    let block = BlockRect::new(0, 6, 2, 2);
    let plan = GeneralIntraResidualPlan::chroma(
        ctx(block, BitDepth::Eight),
        RectChromaPlan::Mode(SupportedChromaMode::Dc, None),
        true,
    )
    .expect("chroma-part plan");

    for plane_id in [PlaneId::U, PlaneId::V] {
        let plane = plan.plane_plan(plane_id).expect("chroma plane");
        assert!(!plane.fsc_mode);
        assert!(!plane.txb_skip_fsc_mode);
        assert_eq!(plane.reconstruction_tx_type, Some(IDTX));
        let mut coeffs = empty_luma_coeffs();
        plane.apply_reconstruction_tx_type(&mut coeffs);
        assert_eq!(coeffs.plane_tx_type, IDTX);
    }
}

#[test]
fn large_luma_chunks_do_not_fill_parent_residual_block() {
    let block = BlockRect::new(0, 0, 32, 16);
    let ctx = ctx(block, BitDepth::Ten);
    let plan = GeneralIntraResidualPlan::rect(
        ctx,
        RectLumaPlan::Dc { use_tcq: true },
        None,
        false,
        None,
        false,
    )
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
fn mrl_chunk_rows_use_transform_local_superblock_boundary() {
    let block = BlockRect::new(32, 0, 16, 32);
    let tx = TxShape::from_luma_4x4(block.width4(), block.height4()).expect("test tx shape");
    let block_ctx = BlockCtx::new(block, tx, 64, 64, BitDepth::Ten, ChromaSampling::Yuv420);
    let plan = GeneralIntraResidualPlan::rect(
        block_ctx,
        RectLumaPlan::MiddleMrl {
            p_angle: 157,
            mrl_index: 3,
            above_mrl_index: 0,
            is_sb_boundary: true,
            secondary_mrl: false,
            use_tcq: false,
        },
        None,
        false,
        None,
        false,
    )
    .expect("64x128 MRL plan");
    let luma: Vec<_> = plan
        .planes
        .iter()
        .filter(|plane| plane.plane_id == PlaneId::Y)
        .copied()
        .collect();

    assert_eq!(luma.len(), 2);
    let mrl_state = |plane: ResidualPlanePlan| match plane.reconstruction {
        ResidualReconstructionPlan::LumaRectMiddleMrl {
            above_mrl_index,
            is_sb_boundary,
            ..
        } => (plane.y, above_mrl_index, is_sb_boundary),
        other => panic!("unexpected reconstruction plan: {other:?}"),
    };
    assert_eq!(mrl_state(luma[0]), (128, 0, true));
    assert_eq!(mrl_state(luma[1]), (192, 3, false));
    assert_eq!(luma[0].y - 1, 127);
    assert_eq!(luma[1].y - 1 - 3, 188);
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
        None,
        false,
    )
    .expect("cfl rect plan");

    assert_large_chroma_order(&plan, true);

    let chroma = plan.plane_plan(PlaneId::U).expect("cfl chroma plane");
    let unit = chroma
        .transform_unit_plan(&PositionedLumaCoeffBlock {
            x: chroma.x,
            y: chroma.y,
            tx_size: TX_4X4,
            middle: false,
            coeffs: empty_luma_coeffs(),
        })
        .expect("cfl transform unit");
    assert_eq!(unit.reconstruction, chroma.reconstruction);
}

#[test]
fn maximum_444_cfl_plan_fits_deferred_plane_capacity() {
    let block = BlockRect::new(0, 0, 64, 64);
    let tx = TxShape::from_luma_4x4(block.width4(), block.height4()).expect("test tx shape");
    let ctx = BlockCtx::new(block, tx, 64, 64, BitDepth::Ten, ChromaSampling::Yuv444);
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
        None,
        false,
    )
    .expect("maximum CFL rect plan");

    assert_eq!(
        plan.planes
            .iter()
            .filter(|plane| plane.defer_reconstruction)
            .count(),
        plan::MAX_DEFERRED_CHROMA_PLANES
    );
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
        None,
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
        None,
        false,
    )
    .expect("non-lossless chroma ref plan");
    let u = plan.plane_plan(PlaneId::U).expect("chroma u");

    assert_eq!((u.x, u.y), (20, 0));
    assert_eq!((u.tx.width_log2(), u.tx.height_log2()), (2, 2));
}

#[test]
fn lossless_chroma_uses_partition_chroma_ref() {
    let block = BlockRect::new(23, 19, 1, 1);
    let chroma_ref = BlockRect::new(22, 16, 4, 2);
    let chroma_tx = TxShape::from_luma_4x4(4, 2).expect("valid chroma reference transform");
    let ctx = ctx(block, BitDepth::Eight).with_chroma_ref(chroma_ref, chroma_tx);
    let plan = GeneralIntraResidualPlan::square(
        ctx,
        IntraLumaPlan::Dc,
        Some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
        false,
        true,
        Some(0),
        true,
    )
    .expect("lossless chroma ref plan");
    let u = plan.plane_plan(PlaneId::U).expect("chroma u");

    assert_eq!((u.x, u.y), (32, 44));
    assert_eq!((u.tx.width4(), u.tx.height4()), (2, 1));
    assert!(u.txb_skip_fsc_mode);
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
        None,
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
        None,
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
fn oversized_explicit_chroma_ref_is_chunked_in_spec_read_order() {
    let block = BlockRect::new(8, 12, 32, 16);
    let tx = TxShape::from_luma_4x4(block.width4(), block.height4()).expect("block shape");
    let ctx = BlockCtx::new(block, tx, 64, 64, BitDepth::Eight, ChromaSampling::Yuv444)
        .with_chroma_ref(block, tx);
    let plan = GeneralIntraResidualPlan::rect(
        ctx,
        RectLumaPlan::Dc { use_tcq: false },
        Some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
        false,
        None,
        false,
    )
    .expect("oversized 4:4:4 residual plan");

    assert_eq!(
        plan.planes
            .iter()
            .map(|plane| (
                plane.plane_id,
                plane.x,
                plane.y,
                plane.tx.width4(),
                plane.tx.height4(),
            ))
            .collect::<Vec<_>>(),
        [
            (PlaneId::Y, 48, 32, 16, 16),
            (PlaneId::U, 48, 32, 16, 16),
            (PlaneId::V, 48, 32, 16, 16),
            (PlaneId::Y, 112, 32, 16, 16),
            (PlaneId::U, 112, 32, 16, 16),
            (PlaneId::V, 112, 32, 16, 16),
        ]
    );
    assert!(
        plan.planes
            .iter()
            .filter(|plane| plane.plane_id != PlaneId::Y)
            .all(|plane| {
                !plane.tx_fills_residual_block()
                    && (plane.residual_width4, plane.residual_height4) == (32, 16)
            })
    );
}

#[test]
fn every_av2_block_shape_maps_to_valid_luma_and_chroma_chunk_tx_sizes() {
    for (block_size, (&width4, &height4)) in NUM_4X4_BLOCKS_WIDE
        .iter()
        .zip(&NUM_4X4_BLOCKS_HIGH)
        .enumerate()
    {
        let width4 = usize::try_from(width4).expect("positive block width");
        let height4 = usize::try_from(height4).expect("positive block height");
        let block = BlockRect::new(0, 0, width4, height4);
        let tx = TxShape::from_luma_4x4(width4, height4).expect("AV2 block shape");
        for chroma in [
            ChromaSampling::Yuv420,
            ChromaSampling::Yuv422,
            ChromaSampling::Yuv444,
        ] {
            for lossless in [false, true] {
                let ctx = BlockCtx::new(block, tx, width4, height4, BitDepth::Eight, chroma)
                    .with_chroma_ref(block, tx);
                let plan = GeneralIntraResidualPlan::rect(
                    ctx,
                    RectLumaPlan::Dc { use_tcq: false },
                    Some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
                    false,
                    None,
                    lossless,
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "block_size={block_size} chroma={chroma:?} lossless={lossless}: {error:?}"
                    )
                });
                for plane in plan.planes.iter().copied() {
                    assert_eq!(
                        (TX_WIDTH_LOG2[plane.tx_size], TX_HEIGHT_LOG2[plane.tx_size],),
                        (plane.tx.width_log2() as i32, plane.tx.height_log2() as i32),
                        "block_size={block_size} chroma={chroma:?} lossless={lossless} plane={:?}",
                        plane.plane_id,
                    );
                }
            }
        }
    }
}

#[test]
fn max_residual_plan_capacity_reuses_storage_after_bound_error() {
    let plan = |width4, height4| {
        let block = BlockRect::new(0, 0, width4, height4);
        let tx = TxShape::from_luma_4x4(width4.min(64), height4.min(64))
            .expect("bounded transform shape");
        let ctx = BlockCtx::new(
            block,
            tx,
            width4,
            height4,
            BitDepth::Eight,
            ChromaSampling::Yuv444,
        );
        GeneralIntraResidualPlan::rect(
            ctx,
            RectLumaPlan::Dc { use_tcq: false },
            Some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
            false,
            None,
            false,
        )
    };

    let max_plan = plan(64, 64).expect("maximum AV2 block plan");
    assert_eq!(max_plan.planes.len(), MAX_RESIDUAL_PLANES);
    let storage = max_plan.planes.as_ptr();
    drop(max_plan);

    let error = plan(65, 64).expect_err("first out-of-table width must exceed the plan bound");
    assert_eq!(error.reason_id(), "general_intra_residual_plane_capacity");

    let after_error = plan(64, 64).expect("bound error must leave plan storage reusable");
    assert_eq!(after_error.planes.len(), MAX_RESIDUAL_PLANES);
    assert_eq!(after_error.planes.as_ptr(), storage);
}

#[test]
fn lossless_v_handoff_uses_final_u_unit_flag() {
    let plane = GeneralIntraResidualPlan::square(
        ctx(BlockRect::new(0, 0, 4, 4), BitDepth::Eight),
        IntraLumaPlan::Dc,
        Some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
        false,
        false,
        None,
        true,
    )
    .expect("lossless plan")
    .plane_plan(PlaneId::U)
    .expect("chroma U plane");
    let unit = |all_zero| {
        let mut coeffs = empty_luma_coeffs();
        coeffs.all_zero = all_zero;
        ParsedTransformUnit {
            block: PositionedLumaCoeffBlock {
                x: plane.x,
                y: plane.y,
                tx_size: plane.tx_size,
                middle: false,
                coeffs,
            },
            palette_color_map: None,
        }
    };
    let parsed = |units| ParsedResidualPlane {
        plane,
        kind: ParsedResidualPlaneKind::Lossless(units),
        cctx_role: CctxRole::None,
    };

    assert!(!parsed(vec![unit(false), unit(true)]).u_nonzero());
    assert!(parsed(vec![unit(true), unit(false)]).u_nonzero());
}

#[test]
fn single_luma_chunk_publication_exposes_top_right_to_later_chunk() {
    let plane = GeneralIntraResidualPlan::square(
        ctx(BlockRect::new(0, 0, 16, 16), BitDepth::Eight),
        IntraLumaPlan::Dc,
        None,
        false,
        false,
        None,
        false,
    )
    .expect("64x64 luma plan")
    .plane_plan(PlaneId::Y)
    .expect("luma plane");
    let parsed = ParsedResidualPlane {
        plane: ResidualPlanePlan {
            x: 64,
            y: 0,
            ..plane
        },
        kind: ParsedResidualPlaneKind::Single {
            coeffs: empty_luma_coeffs(),
            palette_color_map: None,
        },
        cctx_role: CctxRole::None,
    };
    let mut block_decoded =
        TileBlockDecodedState::new(3, 1, 1, 32, 32, 32).expect("block-decoded state");
    block_decoded.clear_superblock(0, 0);
    let later = ctx(BlockRect::new(16, 0, 16, 16), BitDepth::Eight);

    assert_eq!(
        later
            .neighbours_from_block_decoded(PlaneId::Y, &block_decoded)
            .num_above_right(),
        0
    );
    parsed.plane.publish_luma_transform(&mut block_decoded);
    assert_eq!(
        later
            .neighbours_from_block_decoded(PlaneId::Y, &block_decoded)
            .num_above_right(),
        16
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
        None,
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
fn directional_first_partition_handoff_uses_each_transform_units_edges() {
    let block_ctx = ctx(BlockRect::new(0, 0, 16, 16), BitDepth::Eight);
    for mode in [
        SupportedDirectionalLumaMode::D45,
        SupportedDirectionalLumaMode::D67,
        SupportedDirectionalLumaMode::D113,
        SupportedDirectionalLumaMode::D135,
        SupportedDirectionalLumaMode::D157,
        SupportedDirectionalLumaMode::D203,
    ] {
        let plan = GeneralIntraResidualPlan::square(
            block_ctx,
            IntraLumaPlan::DirectionalFirst { mode },
            None,
            false,
            false,
            None,
            false,
        )
        .expect("directional square plan");
        let plane = plan.plane_plan(PlaneId::Y).expect("luma plane");
        let interior = PositionedLumaCoeffBlock {
            x: 4,
            y: 4,
            tx_size: TX_4X4,
            middle: false,
            coeffs: empty_luma_coeffs(),
        };
        let p_angle = crate::prediction::intra::directional_mode_p_angle(mode);
        let reconstruction = if p_angle < 90 {
            ResidualReconstructionPlan::LumaRectOneSidedAbove {
                p_angle,
                use_tcq: false,
            }
        } else if p_angle > 180 {
            ResidualReconstructionPlan::LumaRectOneSidedLeft {
                p_angle,
                use_tcq: false,
            }
        } else {
            ResidualReconstructionPlan::LumaRectMiddle {
                p_angle,
                use_tcq: false,
            }
        };
        assert_eq!(
            plane
                .transform_unit_plan(&interior)
                .expect("partitioned directional transform unit"),
            ResidualPlanePlan {
                x: 4,
                y: 4,
                tx_size: TX_4X4,
                tx: TxShape::from_luma_4x4(1, 1).expect("4x4 tx shape"),
                residual_width4: 1,
                residual_height4: 1,
                zero_corners: false,
                reconstruction,
                block_ctx: ctx(BlockRect::new(1, 1, 1, 1), BitDepth::Eight),
                ..plane
            },
        );

        let origin = PositionedLumaCoeffBlock {
            x: 0,
            y: 0,
            ..interior
        };
        assert_eq!(
            plane
                .transform_unit_plan(&origin)
                .expect("origin transform unit")
                .reconstruction,
            reconstruction,
        );
    }
}

#[test]
fn smooth_first_partition_handoff_replans_the_origin_transform_unit() {
    let block_ctx = ctx(BlockRect::new(0, 0, 16, 16), BitDepth::Eight);
    let plan = GeneralIntraResidualPlan::square(
        block_ctx,
        IntraLumaPlan::NonDcFirst {
            mode: SupportedNonDcLumaMode::Smooth,
        },
        None,
        false,
        false,
        None,
        false,
    )
    .expect("smooth square plan");
    let plane = plan.plane_plan(PlaneId::Y).expect("luma plane");
    let unit = plane
        .transform_unit_plan(&PositionedLumaCoeffBlock {
            x: 0,
            y: 0,
            tx_size: TX_4X4,
            middle: false,
            coeffs: empty_luma_coeffs(),
        })
        .expect("origin transform unit");

    assert_eq!(
        unit.reconstruction,
        ResidualReconstructionPlan::LumaRectSmooth {
            mode: SupportedNonDcLumaMode::Smooth,
            use_tcq: false,
        }
    );
}

#[test]
fn directional_first_partition_handoff_rejects_invalid_transform_geometry() {
    let block_ctx = ctx(BlockRect::new(0, 0, 16, 16), BitDepth::Eight);
    let plan = GeneralIntraResidualPlan::square(
        block_ctx,
        IntraLumaPlan::DirectionalFirst {
            mode: SupportedDirectionalLumaMode::D135,
        },
        None,
        false,
        false,
        None,
        false,
    )
    .expect("directional square plan");
    let plane = plan.plane_plan(PlaneId::Y).expect("luma plane");
    let invalid = PositionedLumaCoeffBlock {
        x: 4,
        y: 4,
        tx_size: usize::MAX,
        middle: false,
        coeffs: empty_luma_coeffs(),
    };

    assert!(matches!(
        plane.transform_unit_plan(&invalid),
        Err(GeneralIntraResidualError::TransformPartitionGeometry {
            table: "Tx_Width_Log2",
            index: usize::MAX,
        })
    ));
}

#[test]
fn active_mrl_directional_plan_uses_transform_unit_wide_angle_mapping() {
    let luma_context = LumaTransformTypeContext::with_mrl_indices(
        crate::bitstream::tile_payload::IntraYMode::D45_PRED_FOR_TEST,
        2,
        2,
        Some(1),
        None,
    );

    for secondary_mrl in [false, true] {
        let plan_for = |width4, height4| {
            mrl_directional_plane(
                width4,
                height4,
                RectLumaPlan::OneSidedAboveMrl {
                    p_angle: 50,
                    mrl_index: 2,
                    above_mrl_index: 0,
                    secondary_mrl,
                    use_tcq: true,
                },
            )
        };

        assert_eq!(
            plan_for(4, 8).unit_directional_replan(luma_context),
            ResidualReconstructionPlan::LumaRectOneSidedLeftMrl {
                p_angle: 230,
                mrl_index: 2,
                above_mrl_index: 0,
                is_sb_boundary: true,
                secondary_mrl,
                use_tcq: true,
            }
        );
        assert_eq!(
            plan_for(8, 8).unit_directional_replan(luma_context),
            ResidualReconstructionPlan::LumaRectOneSidedAboveMrl {
                p_angle: 50,
                mrl_index: 2,
                above_mrl_index: 0,
                secondary_mrl,
                use_tcq: true,
            }
        );
    }
}

#[test]
fn left_mrl_replan_preserves_superblock_boundary_above_line() {
    let plane = mrl_directional_plane(
        8,
        4,
        RectLumaPlan::OneSidedLeftMrl {
            p_angle: 210,
            mrl_index: 1,
            above_mrl_index: 0,
            is_sb_boundary: true,
            secondary_mrl: true,
            use_tcq: false,
        },
    );
    let luma_context = LumaTransformTypeContext::with_mrl_indices(
        crate::bitstream::tile_payload::IntraYMode::D203_PRED_FOR_TEST,
        2,
        1,
        Some(1),
        None,
    );

    assert_eq!(
        plane.unit_directional_replan(luma_context),
        ResidualReconstructionPlan::LumaRectOneSidedAboveMrl {
            p_angle: 30,
            mrl_index: 1,
            above_mrl_index: 0,
            secondary_mrl: true,
            use_tcq: false,
        }
    );
}

#[test]
fn interior_middle_mrl_unit_uses_local_above_line() {
    let plane = mrl_directional_plane(
        8,
        8,
        RectLumaPlan::MiddleMrl {
            p_angle: 135,
            mrl_index: 3,
            above_mrl_index: 0,
            is_sb_boundary: true,
            secondary_mrl: true,
            use_tcq: false,
        },
    );
    let unit = plane
        .transform_unit_plan(&PositionedLumaCoeffBlock {
            x: 0,
            y: 16,
            tx_size: rect_tx_size_from_log2(4, 4).expect("16x16 transform size"),
            middle: false,
            coeffs: empty_luma_coeffs(),
        })
        .expect("interior transform-unit plan");
    let luma_context = LumaTransformTypeContext::with_mrl_indices(
        crate::bitstream::tile_payload::IntraYMode::D135_PRED_FOR_TEST,
        0,
        3,
        Some(1),
        None,
    );

    assert_eq!(
        unit.unit_directional_replan(luma_context),
        ResidualReconstructionPlan::LumaRectMiddleMrl {
            p_angle: 135,
            mrl_index: 3,
            above_mrl_index: 3,
            is_sb_boundary: false,
            secondary_mrl: true,
            use_tcq: false,
        }
    );
}

fn mrl_directional_plane(
    width4: usize,
    height4: usize,
    luma_plan: RectLumaPlan,
) -> ResidualPlanePlan {
    GeneralIntraResidualPlan::rect(
        ctx(BlockRect::new(0, 0, width4, height4), BitDepth::Eight),
        luma_plan,
        None,
        false,
        None,
        false,
    )
    .expect("MRL directional plan")
    .plane_plan(PlaneId::Y)
    .expect("luma plane")
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
            None,
            false,
        )
    } else {
        GeneralIntraResidualPlan::rect(
            ctx,
            RectLumaPlan::Dc { use_tcq: true },
            case.expect_chroma
                .then_some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
            false,
            None,
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
        cctx_type: None,
        plane_tx_type: 0,
        use_tcq: false,
        lossless: false,
    }
}
