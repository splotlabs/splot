// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Inter tile-state construction and error-taxonomy tests.

#![allow(clippy::unwrap_used)]

use super::*;

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
