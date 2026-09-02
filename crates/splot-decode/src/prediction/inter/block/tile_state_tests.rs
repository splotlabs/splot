// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Inter tile-state construction and error-taxonomy tests.

#![allow(clippy::unwrap_used)]

use super::*;
use crate::bitstream::tile_payload::{
    LrUnitRestorationType, WienerNsLrSourceBlock, WienerNsLrUnitFilter,
};

fn assert_invalid_tile_state(error: &crate::DecodeError) {
    assert!(matches!(
        error,
        crate::DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidInterTileConstructionState
        }
    ));
}

fn allocation_error() -> std::collections::TryReserveError {
    Vec::<u8>::new().try_reserve_exact(usize::MAX).unwrap_err()
}

fn assert_workspace_allocation(error: &crate::DecodeError, expected_context: &'static str) {
    assert!(matches!(
        error,
        crate::DecodeError::Reconstruction {
            source: splot_recon::ReconError::WorkspaceAllocationFailed {
                plane: PlaneId::Y,
                context,
            },
        } if *context == expected_context
    ));
}

fn lr_source_block(unit_filter_index: Option<usize>, x: usize) -> WienerNsLrSourceBlock {
    WienerNsLrSourceBlock {
        restoration_type: LrUnitRestorationType::WienerNonsep,
        plane: 0,
        unit_row: 0,
        unit_col: x,
        unit_filter_index,
        tile_mi_row_start: 0,
        tile_mi_row_end: 1,
        tile_mi_col_end: 4,
        x,
        y: 0,
        width: 1,
        height: 1,
        luma_start_x: x,
        luma_end_x: x,
        luma_start_y: 0,
        luma_end_y: 0,
        luma_stripe_start_y: 0,
        luma_stripe_end_y: 0,
    }
}

fn lr_unit_filter(unit_col: usize) -> WienerNsLrUnitFilter {
    WienerNsLrUnitFilter {
        plane: 0,
        unit_row: 0,
        unit_col,
        coeff_count: 18,
        coeffs: [0; 18],
    }
}

#[test]
fn inter_tile_constructor_state_errors_use_typed_header_failure() {
    let coeff_empty =
        TileCoeffContextState::new_for_tile_chroma(0..0, 0..1, ChromaFormatIdc::Yuv444)
            .unwrap_err();
    assert_invalid_tile_state(&inter_tile_coeff_context_error(&coeff_empty));

    let coeff_overflow =
        TileCoeffContextState::new_for_tile_chroma(0..usize::MAX, 0..1, ChromaFormatIdc::Yuv444)
            .unwrap_err();
    assert_invalid_tile_state(&inter_tile_coeff_context_error(&coeff_overflow));

    let segment_empty = TileSegmentIdState::new_for_tile(1..1, 0..1).unwrap_err();
    assert_invalid_tile_state(&inter_tile_segment_id_error(&segment_empty));

    let segment_overflow = TileSegmentIdState::new_for_tile(0..usize::MAX, 0..2).unwrap_err();
    assert_invalid_tile_state(&inter_tile_segment_id_error(&segment_overflow));

    for error in [
        TileBlockDecodedState::new(0, 1, 1, 16, 16, 16).unwrap_err(),
        TileBlockDecodedState::new(3, 1, 1, 0, 16, 16).unwrap_err(),
        TileBlockDecodedState::new(3, 1, 1, usize::MAX, 16, 16).unwrap_err(),
        TileBlockDecodedState::new(3, usize::BITS as usize, 1, 16, 16, 16).unwrap_err(),
        TileBlockDecodedState::new(3, 1, usize::BITS as usize, 16, 16, 16).unwrap_err(),
    ] {
        assert_invalid_tile_state(&inter_tile_block_decoded_error(&error));
    }
}

#[test]
fn inter_tile_constructors_accept_nonempty_boundary_geometry() {
    assert!(
        TileCoeffContextState::new_for_tile_chroma(7..8, 9..10, ChromaFormatIdc::Yuv420).is_ok()
    );
    assert!(TileSegmentIdState::new_for_tile(7..8, 9..10).is_ok());
    assert!(TileBlockDecodedState::new(1, 1, 1, 1, 1, 1).is_ok());
    assert!(TileBlockDecodedState::new(3, 1, 1, 32, 32, 32).is_ok());
    assert!(NeighbourMvGrid::new_for_tile(7..8, 9..10).is_ok());
    assert!(crate::prediction::intra_edge::TileYSmoothGrid::new_for_tile(7..8, 9..10).is_ok());
}

#[test]
fn inter_tile_grid_mapper_exhaustively_separates_state_from_allocation() {
    for error in [
        TileGridConstructionError::EmptyDimensions,
        TileGridConstructionError::ReversedDimensions,
        TileGridConstructionError::AreaOverflow,
    ] {
        assert_invalid_tile_state(&inter_tile_grid_error(&error, "unused"));
    }

    for context in [
        "inter parser MV grid",
        "inter luma smooth grid",
        "inter chroma smooth grid",
        "inter admission MV grid",
    ] {
        assert_workspace_allocation(
            &inter_tile_grid_error(&TileGridConstructionError::Allocation, context),
            context,
        );
    }
}

#[test]
fn inter_tile_constructor_allocation_errors_stay_reconstruction_failures() {
    assert_workspace_allocation(
        &inter_tile_coeff_context_error(&TileCoeffStateError::Allocation(allocation_error())),
        "inter coefficient context state",
    );
    assert_workspace_allocation(
        &inter_tile_segment_id_error(&TileSegmentIdStateError::Allocation {
            source: allocation_error(),
        }),
        "inter segment id state",
    );
    assert_workspace_allocation(
        &inter_tile_block_decoded_error(&TileBlockDecodedStateError::Allocation {
            source: allocation_error(),
        }),
        "inter block decoded state",
    );
}

#[test]
fn loop_restoration_records_rebase_each_tile_once_in_append_order() {
    let mut blocks = vec![lr_source_block(Some(0), 0)];
    let mut filters = vec![lr_unit_filter(0)];

    append_lr_records(
        &mut blocks,
        &mut filters,
        vec![
            lr_source_block(None, 1),
            lr_source_block(Some(0), 2),
            lr_source_block(Some(1), 3),
        ],
        vec![lr_unit_filter(1), lr_unit_filter(2)],
    )
    .unwrap();
    append_lr_records(
        &mut blocks,
        &mut filters,
        vec![lr_source_block(Some(0), 4)],
        vec![lr_unit_filter(3)],
    )
    .unwrap();

    assert_eq!(
        blocks
            .iter()
            .map(|block| block.unit_filter_index)
            .collect::<Vec<_>>(),
        [Some(0), None, Some(1), Some(2), Some(3)]
    );
    assert_eq!(
        filters
            .iter()
            .map(|filter| filter.unit_col)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
}

#[test]
fn invalid_loop_restoration_index_is_typed_and_fail_atomic() {
    let mut blocks = vec![lr_source_block(Some(0), 0)];
    let mut filters = vec![lr_unit_filter(0)];
    let expected_blocks = blocks.clone();
    let expected_filters = filters.clone();

    let error = append_lr_records(
        &mut blocks,
        &mut filters,
        vec![lr_source_block(Some(1), 1)],
        vec![lr_unit_filter(1)],
    )
    .unwrap_err();

    assert!(matches!(
        error,
        crate::DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidLoopRestorationFilterState
        }
    ));
    assert_eq!(blocks, expected_blocks);
    assert_eq!(filters, expected_filters);
    assert!(crate::DecodeDiagnosticReport::from_decode_error(&error).is_none());
}

#[test]
fn superblock_tile_unit_capacity_is_exact_at_frame_and_raster_edges() {
    const MAX_MI_DIMENSION: usize = (1 << 16) / 4;

    assert_eq!(
        tile_unit_capacity(
            &(0..MAX_MI_DIMENSION),
            &(0..MAX_MI_DIMENSION),
            MAX_MI_DIMENSION,
            MAX_MI_DIMENSION,
            16,
        ),
        1_048_577
    );
    assert_eq!(
        tile_unit_capacity(&(0..4), &(0..34), 4, 34, 32),
        3,
        "two effective 128x128 roots plus the terminal unit"
    );
    assert_eq!(
        tile_unit_capacity(
            &(MAX_MI_DIMENSION - 1..MAX_MI_DIMENSION + 32),
            &(MAX_MI_DIMENSION - 1..MAX_MI_DIMENSION + 32),
            MAX_MI_DIMENSION,
            MAX_MI_DIMENSION,
            64,
        ),
        2,
        "a clipped bottom-right edge still contributes one root and one terminal"
    );
}

#[test]
fn superblock_surfaces_follow_raster_order_and_clip_the_frame_edge()
-> core::result::Result<(), Box<dyn std::error::Error>> {
    let workspace = crate::test_support::yuv420_workspace(129, 16, 0);
    let rects = superblock_luma_rects(&(0..4), &(0..34), &workspace, 32)?;

    assert_eq!(
        rects,
        [
            splot_recon::PlaneRect::new(0, 0, 128, 16)?,
            splot_recon::PlaneRect::new(128, 0, 1, 16)?,
        ]
    );
    Ok(())
}

fn surface_scratch(
    info: splot_recon::DecodedFrameInfo,
    rects: &[splot_recon::PlaneRect],
) -> splot_recon::Result<TileDecodeScratch<u8>> {
    let mut surfaces = rects
        .iter()
        .copied()
        .map(|rect| splot_recon::OwnedFrameRect::new(info, rect, 0))
        .collect::<splot_recon::Result<Vec<_>>>()?;
    surfaces.reverse();
    Ok(TileDecodeScratch {
        surfaces,
        ..TileDecodeScratch::default()
    })
}

fn drain_surface_layout(
    scratch: TileDecodeScratch<u8>,
    info: splot_recon::DecodedFrameInfo,
    rects: &[splot_recon::PlaneRect],
) -> splot_recon::Result<Vec<splot_recon::PlaneRect>> {
    let mut source = super::admission::SurfaceSource::new(info, rects.to_vec(), scratch.surfaces);
    let mut handed = Vec::new();
    for unit in 0..rects.len() {
        let Some(surface) = source.take(unit) else {
            break;
        };
        handed.push(surface?.luma_rect());
    }
    Ok(handed)
}

#[test]
fn the_surface_source_hands_out_every_rect_in_raster_order()
-> core::result::Result<(), Box<dyn std::error::Error>> {
    let workspace = crate::test_support::yuv420_workspace(256, 16, 0);
    let info = workspace.info();
    let rects = superblock_luma_rects(&(0..4), &(0..64), &workspace, 32)?;
    let scratch = surface_scratch(info, &rects)?;

    assert_eq!(drain_surface_layout(scratch, info, &rects)?, rects);
    Ok(())
}

#[test]
fn a_returned_surface_is_retargeted_rather_than_reallocated()
-> core::result::Result<(), Box<dyn std::error::Error>> {
    let workspace = crate::test_support::yuv420_workspace(256, 32, 0);
    let info = workspace.info();
    let rects = superblock_luma_rects(&(0..8), &(0..64), &workspace, 32)?;
    let equal_sized: Vec<_> = rects
        .iter()
        .copied()
        .filter(|rect| rect.width() == rects[0].width() && rect.height() == rects[0].height())
        .collect();
    assert!(equal_sized.len() >= 2, "need two same-shaped superblocks");

    let mut source =
        super::admission::SurfaceSource::<u8>::new(info, equal_sized.clone(), Vec::new());
    let first = source.take(0).ok_or("a first surface")??;
    assert_eq!(first.luma_rect(), equal_sized[0]);
    source.give(first);
    assert_eq!(source.free_len(), 1);

    let second = source.take(1).ok_or("a second surface")??;

    assert_eq!(second.luma_rect(), equal_sized[1]);
    assert_eq!(
        source.free_len(),
        0,
        "the returned surface must be retargeted and reused, not left behind"
    );
    Ok(())
}

#[test]
fn incompatible_recycled_surface_layout_is_cleared_whole()
-> core::result::Result<(), Box<dyn std::error::Error>> {
    let workspace = crate::test_support::yuv420_workspace(256, 32, 0);
    let info = workspace.info();
    let rects = superblock_luma_rects(&(0..4), &(0..64), &workspace, 32)?;
    let stale_rects = superblock_luma_rects(&(4..8), &(0..64), &workspace, 32)?;
    let mut scratch = surface_scratch(info, &stale_rects)?;

    scratch.clear_incompatible_surface_layout(info, &rects);

    assert!(scratch.surfaces.is_empty());
    Ok(())
}
