// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Fail-atomic tests for invalid general-intra directional edge state.

use super::*;

#[test]
fn one_sided_edge_state_failures_do_not_mutate_luma_plane() {
    let mut ws =
        new_general_intra_workspace::<u8>(16, 16, BitDepth::Eight, PixelFormat::Yuv420).unwrap();
    ws.write_rect_block(
        PlaneId::Y,
        0,
        0,
        IntraRectBlockSize::new(4, 4).unwrap(),
        &[37; 16 * 16],
    )
    .unwrap();
    let before = ws.samples(PlaneId::Y).unwrap().to_vec();

    assert!(matches!(
        reconstruct_general_intra_one_sided_neighbour_block_into(
            &mut ws,
            &all_zero_luma_block(),
            45,
            PlaneId::Y,
            4,
            0,
            2,
            2,
            0,
            0,
            OneSidedAboveMrl::default(),
            false,
            None,
            None,
            IntraEdgeAvailability {
                above: true,
                left: false,
            },
            BitDepth::Eight,
            OneSidedEdgeFilter::default(),
        ),
        Err(GeneralIntraResidualError::InvalidDirectionalEdgeState)
    ));
    assert_eq!(ws.samples(PlaneId::Y).unwrap(), before);

    assert!(matches!(
        reconstruct_general_intra_one_sided_left_neighbour_block_into(
            &mut ws,
            &all_zero_luma_block(),
            203,
            PlaneId::Y,
            0,
            4,
            2,
            2,
            0,
            0,
            false,
            0,
            0,
            false,
            None,
            None,
            IntraEdgeAvailability {
                above: false,
                left: true,
            },
            BitDepth::Eight,
            OneSidedEdgeFilter::default(),
        ),
        Err(GeneralIntraResidualError::InvalidDirectionalEdgeState)
    ));
    assert_eq!(ws.samples(PlaneId::Y).unwrap(), before);
}

#[test]
fn chroma_mrl_edge_state_failure_does_not_mutate_plane() {
    let mut ws =
        new_general_intra_workspace::<u8>(16, 16, BitDepth::Eight, PixelFormat::Yuv420).unwrap();
    ws.write_rect_block(
        PlaneId::U,
        0,
        0,
        IntraRectBlockSize::new(3, 3).unwrap(),
        &[37; 8 * 8],
    )
    .unwrap();
    let before = ws.samples(PlaneId::U).unwrap().to_vec();

    assert!(matches!(
        reconstruct_general_intra_one_sided_neighbour_block_into(
            &mut ws,
            &all_zero_luma_block(),
            45,
            PlaneId::U,
            0,
            0,
            2,
            2,
            0,
            0,
            OneSidedAboveMrl {
                mrl_index: 1,
                above_mrl_index: 1,
            },
            false,
            None,
            None,
            IntraEdgeAvailability {
                above: false,
                left: false,
            },
            BitDepth::Eight,
            OneSidedEdgeFilter::default(),
        ),
        Err(GeneralIntraResidualError::InvalidDirectionalEdgeState)
    ));
    assert_eq!(ws.samples(PlaneId::U).unwrap(), before);
}

#[test]
fn middle_edge_state_failure_does_not_mutate_luma_plane() {
    let mut ws =
        new_general_intra_workspace::<u8>(16, 16, BitDepth::Eight, PixelFormat::Yuv420).unwrap();
    ws.write_rect_block(
        PlaneId::Y,
        0,
        0,
        IntraRectBlockSize::new(4, 4).unwrap(),
        &[37; 16 * 16],
    )
    .unwrap();
    let before = ws.samples(PlaneId::Y).unwrap().to_vec();

    assert!(matches!(
        reconstruct_general_intra_middle_neighbour_rect_block_into(
            &mut ws,
            &all_zero_luma_block(),
            135,
            PlaneId::Y,
            0,
            0,
            2,
            2,
            0,
            false,
            None,
            None,
            BitDepth::Eight,
            MiddleEdgeAvailability {
                above: true,
                left: false,
            },
            TwoSidedMiddleEdgeFilters {
                above: OneSidedEdgeFilter::default(),
                left: OneSidedEdgeFilter::default(),
            },
        ),
        Err(GeneralIntraResidualError::InvalidDirectionalEdgeState)
    ));
    assert_eq!(ws.samples(PlaneId::Y).unwrap(), before);
}

#[test]
fn dip_edge_state_failure_does_not_mutate_luma_plane() {
    let mut ws =
        new_general_intra_workspace::<u8>(16, 16, BitDepth::Eight, PixelFormat::Yuv420).unwrap();
    ws.write_rect_block(
        PlaneId::Y,
        0,
        0,
        IntraRectBlockSize::new(4, 4).unwrap(),
        &[37; 16 * 16],
    )
    .unwrap();
    let before = ws.samples(PlaneId::Y).unwrap().to_vec();

    assert!(matches!(
        reconstruct_general_intra_luma_dip_rect_block_into(
            &mut ws,
            &all_zero_luma_block(),
            0,
            false,
            4,
            0,
            2,
            2,
            0,
            false,
            0,
            0,
            LumaTransformTypeContext::new(IntraYMode::Dc, 0),
            IntraEdgeAvailability {
                above: true,
                left: false,
            },
            BitDepth::Eight,
        ),
        Err(GeneralIntraResidualError::InvalidDirectionalEdgeState)
    ));
    assert_eq!(ws.samples(PlaneId::Y).unwrap(), before);
}
