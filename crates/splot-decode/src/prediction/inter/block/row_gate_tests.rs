// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bound-derivation, admission and settle-classification tests for
//! [`super::RowReferenceGate`].

#![allow(clippy::expect_used)]

use splot_recon::{
    BitDepth, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneRect, PlaneSize, SubpelPredictParams,
};

use splot_recon::{IDENTITY_WARP_PARAMS, WARPED_BLOCK_SIZE};

use super::super::super::mc::mc_planes;
use super::super::super::mv_scaling::derive_plane_scaling;
use super::super::super::reference::subpel_last_reference_row;
use super::super::super::{BawpSyntax, InterBlock, Mv, PlacedInterBlock, ReconInterpolationFilter};
use super::*;

const FRAME_WIDTH: usize = 1920;
const FRAME_HEIGHT: usize = 1080;

fn info(width: usize, height: usize) -> DecodedFrameInfo {
    DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv420,
        PlaneSize::new(width, height).expect("frame size"),
        PlaneRect::new(0, 0, width, height).expect("visible rect"),
    )
    .expect("frame info")
}

fn cropped_info(width: usize, height: usize, crop_y: usize) -> DecodedFrameInfo {
    DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv420,
        PlaneSize::new(width, height).expect("frame size"),
        PlaneRect::new(0, crop_y, width, height - crop_y).expect("visible rect"),
    )
    .expect("frame info")
}

fn placed(luma_y: usize, luma_h: usize, mv_row: i32) -> PlacedInterBlock {
    PlacedInterBlock {
        luma_x: 0,
        luma_y,
        luma_w: 64,
        luma_h,
        chroma_luma_x: 0,
        chroma_luma_y: luma_y,
        chroma_luma_w: 64,
        chroma_luma_h: luma_h,
        predict_chroma: true,
        sub8x8_chroma: false,
        interintra_chroma: false,
        block: InterBlock {
            ref_frame0: 0,
            ref_frame1: None,
            mv: Mv {
                row: mv_row,
                col: 0,
            },
            mv1: Mv { row: 0, col: 0 },
            interp: ReconInterpolationFilter::EightTap,
            warp_params: [None, None],
            bawp: BawpSyntax::default(),
            interintra: None,
            compound_blend: super::super::super::mc::CompoundBlend::default(),
            optflow_distances: None,
            residual: None,
        },
    }
}

/// The bound one list of one block imposes, with the block's own geometry.
fn published_rows(
    frame: DecodedFrameInfo,
    block: &PlacedInterBlock,
    mv: Mv,
    reach: ListReach,
) -> Option<u32> {
    block_published_rows(
        frame,
        frame,
        block.motion_compensation_rect(),
        block.predict_chroma,
        mv,
        reach,
    )
}

/// The last luma row the prediction of one plane will actually read, derived
/// the way `mc::plane_prediction` derives it.
fn prediction_luma_rows(
    frame: DecodedFrameInfo,
    reference: DecodedFrameInfo,
    block: &PlacedInterBlock,
    mv: Mv,
) -> u32 {
    let rect = block.motion_compensation_rect();
    let reference_size = reference.coded_luma_size();
    let frame_size = frame.coded_luma_size();
    let mut rows = 0u32;
    for (plane, sub_x, sub_y) in mc_planes(frame.pixel_format()) {
        let (plane_x, plane_y, width, height) = rect.plane_rect(plane, sub_x, sub_y);
        let scaling = derive_plane_scaling(
            plane_x as i32,
            plane_y as i32,
            mv.row,
            mv.col,
            sub_x,
            sub_y,
            reference_size.width() as i32,
            reference_size.height() as i32,
            frame_size.width() as i32,
            frame_size.height() as i32,
        );
        let params = SubpelPredictParams {
            interp: splot_recon::InterpolationFilter::EightTap,
            w: width,
            h: height,
            start_x: scaling.start_x,
            start_y: scaling.start_y,
            step_x: scaling.step_x,
            step_y: scaling.step_y,
            first_x: scaling.first_x,
            first_y: scaling.first_y,
            last_x: scaling.last_x,
            last_y: scaling.last_y,
            bit_depth: frame.bit_depth(),
        };
        let last = subpel_last_reference_row(&params);
        rows = rows.max(((last.max(0) as u32).saturating_add(1)) << sub_y);
    }
    rows
}

#[test]
fn a_single_reference_bound_is_the_prediction_bound_over_every_plane() {
    let frame = info(FRAME_WIDTH, FRAME_HEIGHT);
    for (luma_y, luma_h, mv_row) in [
        (64usize, 64usize, 0i32),
        (64, 64, 80),
        (0, 8, -160),
        (1024, 32, 400),
        (512, 128, 3),
    ] {
        let block = placed(luma_y, luma_h, mv_row);
        assert_eq!(
            published_rows(frame, &block, block.block.mv, ListReach::default()),
            Some(prediction_luma_rows(frame, frame, &block, block.block.mv)),
            "block at {luma_y}+{luma_h} mv {mv_row}"
        );
    }
}

#[test]
fn the_chroma_plane_sets_the_bound_of_an_unshifted_block() {
    let frame = info(FRAME_WIDTH, FRAME_HEIGHT);
    let block = placed(64, 64, 0);

    assert_eq!(
        published_rows(frame, &block, block.block.mv, ListReach::default()),
        Some(136),
        "luma reads rows 64..=127 plus the four-tap reach (132 rows), chroma \
         reads rows 32..=63 plus the same reach (68 chroma rows, 136 luma)"
    );
}

#[test]
fn a_cropped_reference_gate_uses_storage_row_coordinates() {
    let frame = info(FRAME_WIDTH, FRAME_HEIGHT);
    let reference = cropped_info(FRAME_WIDTH, FRAME_HEIGHT, 8);
    let block = placed(64, 64, 0);

    assert_eq!(
        block_published_rows(
            frame,
            reference,
            block.motion_compensation_rect(),
            block.predict_chroma,
            block.block.mv,
            ListReach::default(),
        ),
        Some(144),
        "the visible-local 136-row prediction starts eight storage rows down"
    );
}

#[test]
fn a_cropped_reference_gate_offsets_the_bawp_template() {
    let frame = info(FRAME_WIDTH, FRAME_HEIGHT);
    let reference = cropped_info(FRAME_WIDTH, FRAME_HEIGHT, 8);
    let block = placed(64, 8, 8 * 40);
    let bawp = 200;

    assert_eq!(
        block_published_rows(
            frame,
            reference,
            block.motion_compensation_rect(),
            block.predict_chroma,
            block.block.mv,
            ListReach {
                bawp,
                ..ListReach::default()
            },
        ),
        Some(208),
        "the visible-local BAWP bound starts eight storage rows down"
    );
}

#[test]
fn a_downward_motion_vector_moves_the_bound_down_the_reference() {
    let frame = info(FRAME_WIDTH, FRAME_HEIGHT);
    let block = placed(64, 64, 80);

    assert_eq!(
        published_rows(frame, &block, block.block.mv, ListReach::default()),
        Some(146),
        "ten luma samples down is five chroma rows down: 73 chroma rows, 146 luma"
    );
}

#[test]
fn the_bound_never_passes_the_last_reference_row() {
    let frame = info(FRAME_WIDTH, FRAME_HEIGHT);
    let block = placed(FRAME_HEIGHT - 64, 64, 8 * 4096);

    assert_eq!(
        published_rows(frame, &block, block.block.mv, ListReach::default()),
        Some(FRAME_HEIGHT as u32)
    );
}

#[test]
fn a_tip_batch_waits_for_its_first_candidate_over_the_whole_rectangle() {
    let frame = info(FRAME_WIDTH, FRAME_HEIGHT);
    let (past, _past_writer) = RefFrameSlot::<u8>::pending(frame).expect("pending past");
    let (future, _future_writer) = RefFrameSlot::<u8>::pending(frame).expect("pending future");
    for slot in [&past, &future] {
        assert!(slot.progress().expect("progress").begin(&[
            (0, 568),
            (568, 582),
            (582, FRAME_HEIGHT)
        ]));
        slot.progress().expect("progress").publish(0);
    }
    let references = TipReferencePair {
        past_ref: 0,
        future_ref: 1,
        past_offset: -1,
        future_offset: 1,
        ref_offset: 1,
    };
    let temporal = TemporalMvContext::with_tip_sample(
        FRAME_HEIGHT.div_ceil(4),
        FRAME_WIDTH.div_ceil(4),
        references,
        384 / TIP_FIELD_CELL,
        0,
        Mv { row: 361, col: 0 },
    )
    .expect("TIP field");
    let lists = [
        Some(&past),
        Some(&future),
        None,
        None,
        None,
        None,
        None,
        None,
    ];
    let gate = RowReferenceGate {
        lists,
        settle: PixelReferenceGate {
            slots: [None; ReferenceSlot::MAX_SLOTS],
        },
        frame,
        temporal: &temporal,
        tip: Some(references),
    };
    let mut block = placed(384, 128, 0);
    block.luma_w = 128;
    block.chroma_luma_w = 128;
    let mut bounds = RowReferenceBounds::default();

    gate.note_tip(&block, &mut bounds);

    assert_eq!(bounds.needs[1], 582);
    assert_eq!(gate.conditions(&bounds).len(), 2);
}

/// The luma row `splot-recon`'s § 7.13.3.19 block warp reads last, over every
/// 8x8 luma section of one block, derived section by section the way
/// `warp_prediction.rs` derives it: project the section centre through the
/// model, then read seven rows below the projected row.
fn warp_read_luma_rows(block: &PlacedInterBlock, warp_params: [i32; 6]) -> u32 {
    let mut last = 0i64;
    for section_y in (0..block.luma_h).step_by(WARPED_BLOCK_SIZE) {
        for section_x in (0..block.luma_w).step_by(WARPED_BLOCK_SIZE) {
            let source_x = (block.luma_x + section_x + 4) as i64;
            let source_y = (block.luma_y + section_y + 4) as i64;
            let destination = i64::from(warp_params[4]) * source_x
                + i64::from(warp_params[5]) * source_y
                + i64::from(warp_params[1]);
            last = last.max((destination >> 16) + 7);
        }
    }
    (last.max(0) as u32).saturating_add(1)
}

#[test]
fn a_warped_list_bound_dominates_every_section_the_kernel_reads() {
    let frame = info(FRAME_WIDTH, FRAME_HEIGHT);
    for (luma_y, luma_h, shift_rows, shear) in [
        (64usize, 64usize, 0i32, 0i32),
        (64, 64, 12, 0),
        (256, 32, -7, 0),
        (128, 128, 3, 640),
        (512, 64, 40, -768),
    ] {
        let block = placed(luma_y, luma_h, 0);
        let mut warp_params = IDENTITY_WARP_PARAMS;
        warp_params[1] = shift_rows << 16;
        warp_params[4] = shear;
        let reach = ListReach {
            warp: Some(warp_params),
            ..ListReach::default()
        };

        let bound = published_rows(frame, &block, Mv::ZERO, reach).expect("unscaled warp bound");
        let read = warp_read_luma_rows(&block, warp_params);
        assert!(
            bound >= read,
            "block at {luma_y}+{luma_h} shift {shift_rows} shear {shear}: \
             bound {bound} must dominate the {read} rows the kernel reads"
        );
    }
}

#[test]
fn a_scaled_warped_list_stays_unboundable() {
    let frame = info(FRAME_WIDTH, FRAME_HEIGHT);
    let reference = info(FRAME_WIDTH / 2, FRAME_HEIGHT / 2);
    let block = placed(64, 64, 0);

    assert_eq!(
        block_published_rows(
            frame,
            reference,
            block.motion_compensation_rect(),
            block.predict_chroma,
            Mv::ZERO,
            ListReach {
                warp: Some(IDENTITY_WARP_PARAMS),
                ..ListReach::default()
            },
        ),
        None
    );
}

#[test]
fn a_bawp_list_bound_covers_the_template_below_its_reference_position() {
    let frame = info(FRAME_WIDTH, FRAME_HEIGHT);
    let block = placed(64, 64, 0);
    let translation = placed(64, 64, 8 * 40);

    assert_eq!(
        bawp_reference_luma_rows(64, 64, 0),
        80,
        "the template caps at sixteen rows below the block's reference position"
    );
    assert_eq!(
        bawp_reference_luma_rows(64, 8, 0),
        72,
        "a block shorter than the cap only reaches its own height"
    );
    assert_eq!(
        bawp_reference_luma_rows(64, 64, 8 * 40),
        120,
        "a forty-sample motion vector moves the template forty rows down"
    );

    let subpel = published_rows(frame, &block, Mv::ZERO, ListReach::default());
    let with_template = published_rows(
        frame,
        &block,
        Mv::ZERO,
        ListReach {
            bawp: bawp_reference_luma_rows(64, 64, 0),
            ..ListReach::default()
        },
    );
    assert_eq!(subpel, with_template, "the subpel read already covers it");

    let moved = published_rows(
        frame,
        &translation,
        translation.block.mv,
        ListReach {
            bawp: bawp_reference_luma_rows(64, 64, 8 * 40),
            ..ListReach::default()
        },
    );
    assert!(
        moved.is_some_and(|rows| rows >= 120),
        "the template must not be dropped when the subpel read is shorter"
    );
}
