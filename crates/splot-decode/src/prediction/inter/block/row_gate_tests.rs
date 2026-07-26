// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bound-derivation, admission and settle-classification tests for
//! [`super::RowReferenceGate`].

#![allow(clippy::expect_used)]

use splot_recon::{
    BitDepth, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneRect, PlaneSize, SubpelPredictParams,
};

use super::super::super::mc::mc_planes;
use super::super::super::mv_scaling::derive_plane_scaling;
use super::super::super::reference::subpel_last_reference_row;
use super::super::super::{BawpSyntax, InterBlock, Mv, PlacedInterBlock, ReconInterpolationFilter};
use super::super::deferred_recon::{InterReconCommand, PendingKind};
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

fn command(placed: PlacedInterBlock, kind: PendingKind) -> InterReconCommand {
    InterReconCommand::new(placed, kind, 0, splot_core::span::ByteOffset::new(0))
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
            block_published_rows(frame, frame, &block, block.block.mv, false),
            prediction_luma_rows(frame, frame, &block, block.block.mv),
            "block at {luma_y}+{luma_h} mv {mv_row}"
        );
    }
}

#[test]
fn the_chroma_plane_sets_the_bound_of_an_unshifted_block() {
    let frame = info(FRAME_WIDTH, FRAME_HEIGHT);
    let block = placed(64, 64, 0);

    assert_eq!(
        block_published_rows(frame, frame, &block, block.block.mv, false),
        136,
        "luma reads rows 64..=127 plus the four-tap reach (132 rows), chroma \
         reads rows 32..=63 plus the same reach (68 chroma rows, 136 luma)"
    );
}

#[test]
fn a_downward_motion_vector_moves_the_bound_down_the_reference() {
    let frame = info(FRAME_WIDTH, FRAME_HEIGHT);
    let block = placed(64, 64, 80);

    assert_eq!(
        block_published_rows(frame, frame, &block, block.block.mv, false),
        146,
        "ten luma samples down is five chroma rows down: 73 chroma rows, 146 luma"
    );
}

#[test]
fn the_bound_never_passes_the_last_reference_row() {
    let frame = info(FRAME_WIDTH, FRAME_HEIGHT);
    let block = placed(FRAME_HEIGHT - 64, 64, 8 * 4096);

    assert_eq!(
        block_published_rows(frame, frame, &block, block.block.mv, false),
        FRAME_HEIGHT as u32
    );
}

#[test]
fn unboundable_readers_are_classified_by_their_reason() {
    let mut warped = placed(64, 64, 0);
    warped.block.warp_params[0] = Some([0; 6]);
    let mut bawp = placed(64, 64, 0);
    bawp.block.bawp.enabled = true;

    assert_eq!(
        settle_reason(&command(placed(64, 64, 0), PendingKind::Single)),
        None
    );
    assert_eq!(
        settle_reason(&command(warped, PendingKind::Single)),
        Some(SettleReason::Warp)
    );
    assert_eq!(
        settle_reason(&command(bawp, PendingKind::Single)),
        Some(SettleReason::Bawp)
    );
    assert_eq!(
        settle_reason(&command(placed(64, 64, 0), PendingKind::Tip)),
        Some(SettleReason::Tip)
    );
}

#[test]
fn a_row_is_admitted_once_its_lists_have_published_their_rows() {
    let frame = info(64, 192);
    let (first, _first_writer) = RefFrameSlot::<u8>::pending(frame).expect("pending slot");
    let (second, _second_writer) = RefFrameSlot::<u8>::pending(frame).expect("pending slot");
    for slot in [&first, &second] {
        assert!(
            slot.progress()
                .expect("progress")
                .begin(&[(0, 64), (64, 128), (128, 192)])
        );
    }
    let lists = [
        Some(&first),
        Some(&second),
        None,
        None,
        None,
        None,
        None,
        None,
    ];
    let mut needs = [0u32; MAX_LISTS];
    needs[0] = 64;
    needs[1] = 128;
    let bounds = RowReferenceBounds {
        needs,
        settle: false,
    };

    assert!(!lists_published(&lists, &bounds), "nothing published yet");
    first.progress().expect("progress").publish(0);
    assert!(
        !lists_published(&lists, &bounds),
        "the second list is still short"
    );
    second.progress().expect("progress").publish(0);
    assert!(
        !lists_published(&lists, &bounds),
        "the second list needs two stripes"
    );
    second.progress().expect("progress").publish(1);
    assert!(lists_published(&lists, &bounds), "both lists are covered");
}

#[test]
fn a_list_a_row_never_reads_imposes_no_requirement() {
    let frame = info(64, 192);
    let (slot, _writer) = RefFrameSlot::<u8>::pending(frame).expect("pending slot");
    let lists = [None, Some(&slot), None, None, None, None, None, None];

    assert!(lists_published(
        &lists,
        &RowReferenceBounds {
            needs: [0; MAX_LISTS],
            settle: false,
        }
    ));
}
