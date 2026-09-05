// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::bitstream::tile_payload::{LumaTransformPartitionUnits, TileBlockDecodedState};
use crate::tile::block_context::{BlockRect, ChromaSampling, TxShape};
use splot_core::symbol::Symbol;
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
        },
        Case {
            label: "square chroma-u 10-bit",
            rect: BlockRect::new(0, 0, 16, 16),
            bit_depth: BitDepth::Ten,
            plane: PlaneId::U,
            expected_tx_log2: (5, 5),
        },
        Case {
            label: "square chroma-v dependency",
            rect: BlockRect::new(0, 0, 16, 16),
            bit_depth: BitDepth::Eight,
            plane: PlaneId::V,
            expected_tx_log2: (5, 5),
        },
        Case {
            label: "rect luma",
            rect: BlockRect::new(0, 0, 16, 8),
            bit_depth: BitDepth::Eight,
            plane: PlaneId::Y,
            expected_tx_log2: (6, 5),
        },
        Case {
            label: "rect chroma-u",
            rect: BlockRect::new(0, 0, 16, 8),
            bit_depth: BitDepth::Ten,
            plane: PlaneId::U,
            expected_tx_log2: (5, 4),
        },
        Case {
            label: "rect chroma-v dependency",
            rect: BlockRect::new(0, 0, 16, 8),
            bit_depth: BitDepth::Eight,
            plane: PlaneId::V,
            expected_tx_log2: (5, 4),
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
    let plan = GeneralIntraResidualPlan::rect(
        ctx,
        RectLumaPlan::Dc { use_tcq: false },
        Some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
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
    let tx_8x8 = splot_recon::tx_size_index(3, 3).expect("8x8 transform");
    let mut blocks = Vec::new();
    let mut chroma_blocks = crate::filters::deblock::ChromaDeblockRecords::default();
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

    let u: Vec<_> = chroma_blocks
        .iter_plane(0)
        .map(|(_, record)| record)
        .collect();
    let v: Vec<_> = chroma_blocks
        .iter_plane(1)
        .map(|(_, record)| record)
        .collect();
    assert_eq!(u.len(), 2);
    assert_eq!(u[0].chroma_base_c, 0);
    assert_eq!(u[1].chroma_base_c, 4);
    assert_eq!(u[1].n4w, 4);
    assert_eq!(u[1].chroma_base_r, 2);
    assert_eq!(u[1].n4h, 2);
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].qindex, 37);
    assert!(v[0].lossless);
}

#[test]
fn chroma_dpcm_direction_is_preserved_for_both_planes() {
    let block = BlockRect::new(0, 0, 16, 16);
    let ctx = ctx(block, BitDepth::Eight);
    let plan = GeneralIntraResidualPlan::rect(
        ctx,
        RectLumaPlan::Dc { use_tcq: false },
        Some(RectChromaPlan::Mode(
            SupportedChromaMode::Vertical,
            Some(DpcmDirection::Vertical),
        )),
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
    let plan = GeneralIntraResidualPlan::rect(
        ctx,
        RectLumaPlan::Dc { use_tcq: true },
        Some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
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
        ResidualReconstructionPlan::Luma(RectLumaPlan::MiddleMrl {
            above_mrl_index,
            is_sb_boundary,
            ..
        }) => (plane.y, above_mrl_index, is_sb_boundary),
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
            params: CflParams::DerivedAlpha,
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
            params: CflParams::DerivedAlpha,
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
    let plan = GeneralIntraResidualPlan::rect(
        ctx,
        RectLumaPlan::Dc { use_tcq: false },
        Some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
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
fn max_residual_plan_capacity_reuses_storage_after_invalid_geometry() {
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
    assert_eq!(error, ResidualPlanError::InvalidGeometry);

    let after_error = plan(64, 64).expect("bound error must leave plan storage reusable");
    assert_eq!(after_error.planes.len(), MAX_RESIDUAL_PLANES);
    assert_eq!(after_error.planes.as_ptr(), storage);
}

#[test]
fn residual_plan_storage_allocation_is_fallible() {
    GeneralIntraResidualPlan::take(usize::MAX)
        .expect_err("an impossible capacity must fail without allocating");
}

#[test]
fn transform_size_lookup_rejects_internal_shapes_outside_av2_table() {
    for (tx_size, (&width_log2, &height_log2)) in
        TX_WIDTH_LOG2.iter().zip(&TX_HEIGHT_LOG2).enumerate()
    {
        let width_log2 = u32::try_from(width_log2).expect("non-negative AV2 transform width");
        let height_log2 = u32::try_from(height_log2).expect("non-negative AV2 transform height");
        assert_eq!(
            splot_recon::tx_size_index(width_log2, height_log2),
            Ok(tx_size)
        );
    }
    assert!(splot_recon::tx_size_index(7, 7).is_err());
    assert!(splot_recon::tx_size_index(8, 8).is_err());
}

#[test]
fn lossless_v_handoff_uses_final_u_unit_flag() {
    let plane = GeneralIntraResidualPlan::rect(
        ctx(BlockRect::new(0, 0, 4, 4), BitDepth::Eight),
        RectLumaPlan::Dc { use_tcq: false },
        Some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
        false,
        None,
        true,
    )
    .expect("lossless plan")
    .plane_plan(PlaneId::U)
    .expect("chroma U plane");
    let unit = |all_zero: bool| {
        let mut coeffs = empty_luma_coeffs();
        if !all_zero {
            coeffs.eob = 1;
            coeffs.quant_range = 0..16;
        }
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
    let plane = GeneralIntraResidualPlan::rect(
        ctx(BlockRect::new(0, 0, 16, 16), BitDepth::Eight),
        RectLumaPlan::Dc { use_tcq: false },
        None,
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
    let luma = LumaTransformTypeContext::new(crate::bitstream::tile_payload::IntraYMode::D135, -3);

    assert_eq!(
        chroma_angle_delta_uv(
            PlaneId::U,
            crate::bitstream::tile_payload::IntraYMode::D135.value(),
            luma,
        ),
        -3
    );
    assert_eq!(
        chroma_angle_delta_uv(
            PlaneId::V,
            crate::bitstream::tile_payload::IntraYMode::Dc.value(),
            luma
        ),
        0
    );
    assert_eq!(
        chroma_angle_delta_uv(
            PlaneId::Y,
            crate::bitstream::tile_payload::IntraYMode::D135.value(),
            luma,
        ),
        0
    );
}

#[test]
fn palette_map_is_sliced_for_each_partitioned_transform_size() {
    let block = BlockRect::new(2, 3, 16, 16);
    let ctx = ctx(block, BitDepth::Ten);
    let palette = LumaPalette::from_size_symbol(Symbol::new(2), [16, 64, 128, 240, 0, 0, 0, 0]);
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
fn invalid_palette_unit_geometry_is_reconstruction_state() {
    let ctx = ctx(BlockRect::new(2, 3, 16, 16), BitDepth::Ten);
    let palette = LumaPalette::from_size_symbol(Symbol::new(2), [16, 64, 128, 240, 0, 0, 0, 0]);
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
    let parent_map = vec![3; parent_width * parent_height];

    for (x, y, expected_context) in [
        (plane.x - 1, plane.y, "palette transform X origin"),
        (plane.x, plane.y - 1, "palette transform Y origin"),
        (plane.x + parent_width, plane.y, "palette transform extent"),
    ] {
        let block = PositionedLumaCoeffBlock {
            x,
            y,
            tx_size: 0,
            middle: false,
            coeffs: empty_luma_coeffs(),
        };
        assert!(matches!(
            plane.palette_color_map_for_unit(Some(&parent_map), &block),
            Err(GeneralIntraResidualError::InvalidReconstructionState { context })
                if context == expected_context
        ));
    }
}

#[test]
fn smooth_first_partition_handoff_replans_the_origin_transform_unit() {
    let block_ctx = ctx(BlockRect::new(0, 0, 16, 16), BitDepth::Eight);
    let plan = GeneralIntraResidualPlan::rect(
        block_ctx,
        RectLumaPlan::Smooth {
            mode: SupportedNonDcLumaMode::Smooth,
            use_tcq: false,
        },
        None,
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
        ResidualReconstructionPlan::Luma(RectLumaPlan::Smooth {
            mode: SupportedNonDcLumaMode::Smooth,
            use_tcq: false,
        })
    );
}

#[test]
fn four_way_partition_clipped_to_one_unit_keeps_unit_reconstruction_and_deblock_geometry() {
    let block = BlockRect::new(8, 8, 16, 16);
    let block_ctx = BlockCtx::new(
        block,
        TxShape::from_luma_4x4(block.width4(), block.height4())
            .expect("64x64 block has a transform shape"),
        16,
        16,
        BitDepth::Eight,
        ChromaSampling::Yuv420,
    );
    let plane = GeneralIntraResidualPlan::rect(
        block_ctx,
        RectLumaPlan::Dc { use_tcq: false },
        None,
        false,
        None,
        false,
    )
    .expect("64x64 residual plan is valid")
    .plane_plan(PlaneId::Y)
    .expect("luma plane is present");
    let tx_32x32 = splot_recon::tx_size_index(5, 5).expect("32x32 has a transform index");
    let unit = |x, y| PositionedLumaCoeffBlock {
        x,
        y,
        tx_size: tx_32x32,
        middle: false,
        coeffs: empty_luma_coeffs(),
    };
    let partition =
        LumaTransformPartitionUnits::four([unit(32, 32), unit(64, 32), unit(32, 64), unit(64, 64)]);
    let original_count = partition.len();
    let visible = partition
        .try_filter_map::<_, ()>(|unit| Ok((unit.x < 64 && unit.y < 64).then_some(unit)))
        .expect("infallible clipping predicate");
    assert_eq!(original_count, 4);
    assert_eq!(visible.len(), 1);
    let mut deblock_blocks = Vec::new();
    let mut chroma_blocks = crate::filters::deblock::ChromaDeblockRecords::new();
    let mut tx_skip_records = Vec::new();
    let mut deblock = DeblockRecorder {
        blocks: &mut deblock_blocks,
        chroma_blocks: &mut chroma_blocks,
        tx_skip_records: &mut tx_skip_records,
        block_r: 8,
        block_c: 8,
        chroma_tx: None,
        chroma_subsampling: (1, 1),
        qindex: 0,
        lossless: false,
    };

    let parsed = plane
        .retain_partitioned_luma(visible, None, &mut deblock)
        .expect("visible transform unit has valid geometry");
    assert_eq!(deblock_blocks.len(), 1);
    assert_eq!(
        (
            deblock_blocks[0].r,
            deblock_blocks[0].c,
            deblock_blocks[0].n4w,
            deblock_blocks[0].n4h,
            deblock_blocks[0].luma_tx,
        ),
        (8, 8, 8, 8, tx_32x32)
    );
    assert_eq!(
        tx_skip_records
            .iter()
            .map(|record| (record.row, record.col, record.rows, record.cols))
            .collect::<Vec<_>>(),
        [(8, 8, 8, 8)]
    );

    let mut workspace = crate::pipeline::reconstruct::new_general_intra_workspace::<u8>(
        96,
        96,
        BitDepth::Eight,
        splot_recon::PixelFormat::Yuv420,
    )
    .expect("96x96 workspace is valid");
    workspace
        .write_rect_block(
            PlaneId::Y,
            32,
            28,
            splot_recon::IntraRectBlockSize::new(5, 2).expect("32x4 is valid"),
            &[192; 128],
        )
        .expect("above reference write is in bounds");
    workspace
        .write_rect_block(
            PlaneId::Y,
            28,
            32,
            splot_recon::IntraRectBlockSize::new(2, 5).expect("4x32 is valid"),
            &[64; 128],
        )
        .expect("left reference write is in bounds");
    let mut block_decoded =
        TileBlockDecodedState::new(3, 1, 1, 16, 24, 24).expect("tile state is valid");
    block_decoded.clear_superblock(0, 0);
    let mut expected_decoded = block_decoded.clone();
    expected_decoded.set_luma_transform(32, 32, 8, 8);
    parsed
        .reconstruct(
            &mut crate::pipeline::general_intra::GeneralIntraReconScratch::default(),
            &mut workspace,
            &mut block_decoded,
            &[],
            0,
            crate::prediction::intra_edge::IntraEdgeCtx {
                enable_ibp: false,
                enable_intra_edge_filter: false,
                above_smooth: false,
                left_smooth: false,
                chroma_above_smooth: false,
                chroma_left_smooth: false,
            },
            LumaTransformTypeContext::new(crate::bitstream::tile_payload::IntraYMode::Dc, 0),
        )
        .expect("visible transform unit reconstructs");

    assert_eq!(block_decoded, expected_decoded);
    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 32, 32), Ok(128));
    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 63, 63), Ok(128));
    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 64, 32), Ok(0));
}

#[test]
fn first_partition_handoff_rejects_invalid_transform_geometry() {
    let block_ctx = ctx(BlockRect::new(0, 0, 16, 16), BitDepth::Eight);
    let plan = GeneralIntraResidualPlan::rect(
        block_ctx,
        RectLumaPlan::Smooth {
            mode: SupportedNonDcLumaMode::Smooth,
            use_tcq: false,
        },
        None,
        false,
        None,
        false,
    )
    .expect("smooth square plan");
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
fn chroma_directional_plan_uses_transform_unit_wide_angle_mapping() {
    let plan = GeneralIntraResidualPlan::rect(
        ctx(BlockRect::new(0, 0, 4, 16), BitDepth::Ten),
        RectLumaPlan::Dc { use_tcq: false },
        Some(RectChromaPlan::Directional {
            mode: SupportedChromaMode::D67,
            angle_delta_uv: 0,
            dpcm: None,
        }),
        false,
        None,
        true,
    )
    .expect("lossless rectangular plan");
    let plane = plan.plane_plan(PlaneId::U).expect("chroma plane");
    let luma_context =
        LumaTransformTypeContext::new(crate::bitstream::tile_payload::IntraYMode::Dc, 0);

    assert_eq!(
        crate::pipeline::general_intra::wide_angle_mapped_p_angle(8, 32, 67),
        247
    );
    assert_eq!(
        plane.unit_directional_replan(luma_context),
        ResidualReconstructionPlan::ChromaOneSided(247, None)
    );

    let unit = plane
        .transform_unit_plan(&PositionedLumaCoeffBlock {
            x: plane.x,
            y: plane.y,
            tx_size: TX_4X4,
            middle: false,
            coeffs: empty_luma_coeffs(),
        })
        .expect("4x4 chroma transform-unit plan");
    assert_eq!((unit.tx.width_log2(), unit.tx.height_log2()), (2, 2));
    assert_eq!(
        unit.unit_directional_replan(luma_context),
        ResidualReconstructionPlan::ChromaOneSided(67, None)
    );
}

#[test]
fn active_mrl_directional_plan_uses_transform_unit_wide_angle_mapping() {
    let luma_context = LumaTransformTypeContext::with_mrl_indices(
        crate::bitstream::tile_payload::IntraYMode::D45,
        2,
        2,
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
            ResidualReconstructionPlan::Luma(RectLumaPlan::OneSidedLeftMrl {
                p_angle: 230,
                mrl_index: 2,
                above_mrl_index: 0,
                is_sb_boundary: true,
                secondary_mrl,
                use_tcq: true,
            })
        );
        assert_eq!(
            plan_for(8, 8).unit_directional_replan(luma_context),
            ResidualReconstructionPlan::Luma(RectLumaPlan::OneSidedAboveMrl {
                p_angle: 50,
                mrl_index: 2,
                above_mrl_index: 0,
                secondary_mrl,
                use_tcq: true,
            })
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
        crate::bitstream::tile_payload::IntraYMode::D203,
        2,
        1,
        None,
    );

    assert_eq!(
        plane.unit_directional_replan(luma_context),
        ResidualReconstructionPlan::Luma(RectLumaPlan::OneSidedAboveMrl {
            p_angle: 30,
            mrl_index: 1,
            above_mrl_index: 0,
            secondary_mrl: true,
            use_tcq: false,
        })
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
            tx_size: splot_recon::tx_size_index(4, 4).expect("16x16 transform size"),
            middle: false,
            coeffs: empty_luma_coeffs(),
        })
        .expect("interior transform-unit plan");
    let luma_context = LumaTransformTypeContext::with_mrl_indices(
        crate::bitstream::tile_payload::IntraYMode::D135,
        0,
        3,
        None,
    );

    assert_eq!(
        unit.unit_directional_replan(luma_context),
        ResidualReconstructionPlan::Luma(RectLumaPlan::MiddleMrl {
            p_angle: 135,
            mrl_index: 3,
            above_mrl_index: 3,
            is_sb_boundary: false,
            secondary_mrl: true,
            use_tcq: false,
        })
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
    let plan = GeneralIntraResidualPlan::rect(
        ctx,
        RectLumaPlan::Dc { use_tcq: true },
        Some(RectChromaPlan::Mode(SupportedChromaMode::Dc, None)),
        false,
        None,
        false,
    )
    .unwrap_or_else(|error| panic!("{}: {error}", case.label));
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
        eob: 0,
        quant_range: 0..0,
        intra_ist: None,
        cctx_type: None,
        plane_tx_type: 0,
        use_tcq: false,
        lossless: false,
    }
}

/// A block whose above neighbour lies outside the tile must reconstruct from the
/// §7.13.2.1 fallback edge, so the reconstructed samples cannot depend on whatever
/// the out-of-tile row happens to hold.
#[test]
fn rect_luma_reconstruct_masks_tile_unavailable_above_edge() {
    let plans = [
        RectLumaPlan::Dc { use_tcq: false },
        RectLumaPlan::Smooth {
            mode: SupportedNonDcLumaMode::SmoothVertical,
            use_tcq: false,
        },
    ];
    for plan in plans {
        let low_above = tile_top_reconstruction_samples(plan, 7);
        let high_above = tile_top_reconstruction_samples(plan, 240);
        assert_eq!(
            low_above, high_above,
            "rect luma {plan:?} must ignore the tile-unavailable above edge"
        );
    }
}

fn tile_top_reconstruction_samples(plan: RectLumaPlan, above: u8) -> Vec<u8> {
    let block_ctx = tile_top_block_ctx();
    let mut workspace = workspace_with_tile_boundary_edges(above);
    let block_decoded = TileBlockDecodedState::new(3, 1, 1, 16, 16, 16).expect("block decoded");
    let plane = GeneralIntraResidualPlan::rect(block_ctx, plan, None, false, None, false)
        .expect("rect luma plan")
        .plane_plan(PlaneId::Y)
        .expect("luma plane");

    plane
        .reconstruct(
            &mut crate::pipeline::general_intra::GeneralIntraReconScratch::default(),
            &mut workspace,
            empty_luma_coeffs().view(&[]),
            &block_decoded,
            None,
            0,
            crate::prediction::intra_edge::IntraEdgeCtx {
                enable_ibp: false,
                enable_intra_edge_filter: false,
                above_smooth: false,
                left_smooth: false,
                chroma_above_smooth: false,
                chroma_left_smooth: false,
            },
            LumaTransformTypeContext::new(crate::bitstream::tile_payload::IntraYMode::Dc, 0),
        )
        .expect("tile-top luma reconstruction");

    (0..8)
        .flat_map(|row| (0..8).map(move |col| (row, col)))
        .map(|(row, col)| {
            workspace
                .reconstructed_sample(PlaneId::Y, 8 + col, 8 + row)
                .expect("reconstructed sample")
        })
        .collect()
}

fn tile_top_block_ctx() -> BlockCtx {
    BlockCtx::new(
        BlockRect::new(2, 2, 2, 2),
        TxShape::from_luma_4x4(2, 2).expect("2x2 tx shape"),
        16,
        16,
        BitDepth::Eight,
        ChromaSampling::Yuv420,
    )
    .with_tile_bounds(2, 0, 16)
}

fn workspace_with_tile_boundary_edges(above: u8) -> splot_recon::CurrentFrameWorkspace<u8> {
    let mut workspace = crate::pipeline::reconstruct::new_general_intra_workspace::<u8>(
        64,
        64,
        BitDepth::Eight,
        splot_recon::PixelFormat::Yuv420,
    )
    .expect("workspace");
    workspace
        .write_rect_block(
            PlaneId::Y,
            8,
            4,
            splot_recon::IntraRectBlockSize::new(3, 2).expect("above block size"),
            &[above; 32],
        )
        .expect("above block");
    let mut left_block = vec![0u8; 4 * 8];
    for (row, sample) in [40u8, 45, 50, 55, 60, 65, 70, 75].into_iter().enumerate() {
        for col in 0..4 {
            left_block[row * 4 + col] = sample;
        }
    }
    workspace
        .write_rect_block(
            PlaneId::Y,
            4,
            8,
            splot_recon::IntraRectBlockSize::new(2, 3).expect("left block size"),
            &left_block,
        )
        .expect("left block");
    workspace
}
