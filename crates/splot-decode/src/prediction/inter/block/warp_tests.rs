// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::IDENTITY_WARP_PARAMS;

use super::super::super::Mv;
use super::super::super::find_mv_stack::{MvBlockContext, reduce_warp_model};
use super::super::resolve::WarpDeltaSyntax;
use super::super::{WARP_PARAM_REDUCE_BITS, WARPEDMODEL_PREC_BITS, WARPEDMODEL_TRANS_CLAMP};
use super::{
    apply_warp_delta, local_warp_estimation, set_warp_translation, warp_center, warp_delta_scale,
    warp_round2, wedge_index,
};
use crate::{DecodeHeaderStateError, error::DecodeError};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn is_warp_state(error: &DecodeError) -> bool {
    matches!(
        error,
        DecodeError::HeaderState {
            source: DecodeHeaderStateError::InvalidInterWarpModelState,
        }
    )
}

#[test]
fn extend_warp_round2_uses_ordinary_negative_ties_and_clamps_to_i32() {
    assert_eq!(warp_round2(-3, 1), -1);
    assert_eq!(warp_round2(-1, 1), 0);
    assert_eq!(warp_round2(1, 1), 1);
    assert_eq!(warp_round2(3, 1), 2);

    assert_eq!(warp_round2(i64::MIN, 0), i32::MIN);
    assert_eq!(warp_round2(i64::MAX, 0), i32::MAX);
    assert_eq!(warp_round2(i64::from(i32::MAX) * 2 + 1, 1), i32::MAX);
    assert_eq!(warp_round2(i64::from(i32::MIN) * 2 - 1, 1), i32::MIN);
}

#[test]
fn reduce_warp_model_is_total_at_every_i32_endpoint() {
    let mut minimum = [0, 0, i32::MIN, i32::MIN, i32::MIN, i32::MIN];
    reduce_warp_model(&mut minimum);
    assert_eq!(minimum, [0, 0, 32_832, -32_704, -32_704, 32_832]);

    let mut maximum = [0, 0, i32::MAX, i32::MAX, i32::MAX, i32::MAX];
    reduce_warp_model(&mut maximum);
    assert_eq!(maximum, [0, 0, 98_240, 32_704, 32_704, 98_240]);
}

#[test]
fn every_wedge_angle_distance_permutation_maps_once() {
    let mut indices = Vec::new();
    for angle in 0..20 {
        for dist in 0..4 {
            if let Ok(index) = wedge_index(angle, dist) {
                indices.push(index);
            }
        }
    }
    assert_eq!(indices, (0..68).collect::<Vec<_>>());
}

#[test]
fn invalid_wedge_angle_distance_state_is_typed_separately() {
    for (angle, dist) in [(20, 0), (0, 4), (0, 0), (5, 0), (10, 0)] {
        assert!(matches!(
            wedge_index(angle, dist),
            Err(DecodeError::HeaderState {
                source: DecodeHeaderStateError::InvalidInterWedgeState,
            })
        ));
    }
}

#[test]
fn warp_delta_scaling_reaches_the_av2_extrema_without_overflow() {
    assert_eq!(warp_delta_scale(7, false, false), 14_336);
    assert_eq!(warp_delta_scale(7, true, false), -14_336);
    assert_eq!(warp_delta_scale(14, false, true), 14_336);
    assert_eq!(warp_delta_scale(14, true, true), -14_336);
}

#[test]
fn warp_geometry_rejects_zero_and_usize_max_as_typed_state() -> TestResult {
    assert_eq!(warp_center(0, 1)?, 1);
    for (mi, n4) in [
        (0, 0),
        (1, 0),
        (usize::MAX, 1),
        (0, usize::MAX),
        (usize::MAX / 32, 1),
    ] {
        let Err(error) = warp_center(mi, n4) else {
            return Err("invalid warp geometry was accepted".into());
        };
        assert!(is_warp_state(&error));
    }

    let original = IDENTITY_WARP_PARAMS;
    for (mi_row, mi_col, n4w, n4h) in [
        (0, 0, 0, 1),
        (0, 0, 1, 0),
        (usize::MAX, 0, 1, 1),
        (0, usize::MAX, 1, 1),
    ] {
        let mut params = original;
        let Err(error) = set_warp_translation(&mut params, Mv::ZERO, mi_row, mi_col, n4w, n4h)
        else {
            return Err("invalid translation geometry was accepted".into());
        };
        assert!(is_warp_state(&error));
        assert_eq!(params, original, "translation rejection must be atomic");
    }
    Ok(())
}

#[test]
fn maximum_av2_geometry_mv_and_slopes_produce_clamped_translation() -> TestResult {
    const MAX_AV2_DIMENSION: usize = 1 << 16;
    const MAX_BLOCK_N4: usize = 32;
    let mi = (MAX_AV2_DIMENSION - MAX_BLOCK_N4 * 4) / 4;
    let mut params = [0, 0, i32::MAX, i32::MIN, i32::MAX, i32::MIN];
    set_warp_translation(
        &mut params,
        Mv {
            row: (1 << 16) - 1,
            col: -(1 << 16) + 1,
        },
        mi,
        mi,
        MAX_BLOCK_N4,
        MAX_BLOCK_N4,
    )?;

    let high = WARPEDMODEL_TRANS_CLAMP - (1 << WARP_PARAM_REDUCE_BITS);
    assert!((-WARPEDMODEL_TRANS_CLAMP..=high).contains(&params[0]));
    assert!((-WARPEDMODEL_TRANS_CLAMP..=high).contains(&params[1]));
    Ok(())
}

#[test]
fn maximum_av2_local_least_squares_state_remains_total() -> TestResult {
    const MAX_AV2_DIMENSION: usize = 1 << 16;
    const MAX_BLOCK_N4: usize = 32;
    let mi = (MAX_AV2_DIMENSION - MAX_BLOCK_N4 * 4) / 4;
    let mid = warp_center(mi, MAX_BLOCK_N4)?;
    let source = mid * 8;
    let mv = Mv {
        row: (1 << 16) - 1,
        col: -(1 << 16) + 1,
    };
    let samples = [
        [source - 64, source, source - 64 + mv.row, source + mv.col],
        [source, source - 64, source + mv.row, source - 64 + mv.col],
        [source + 64, source, source + 64 + mv.row, source + mv.col],
        [source, source + 64, source + mv.row, source + 64 + mv.col],
    ];
    let params = local_warp_estimation(&samples, mv, mi, mi, MAX_BLOCK_N4, MAX_BLOCK_N4)?;

    let max_slope = (1 << (WARPEDMODEL_PREC_BITS - 1)) - (1 << WARP_PARAM_REDUCE_BITS);
    for (index, &param) in params.iter().enumerate().skip(2) {
        let offset = if index == 2 || index == 5 {
            1 << WARPEDMODEL_PREC_BITS
        } else {
            0
        };
        assert!((-max_slope..=max_slope).contains(&(param - offset)));
    }
    Ok(())
}

fn block() -> MvBlockContext {
    MvBlockContext {
        mi_row: 0,
        mi_col: 0,
        bw4: 2,
        bh4: 2,
        sb_h4: 16,
        ref_frame0: 0,
        ref_frame1: None,
        mi_rows: 16,
        mi_cols: 16,
    }
}

#[test]
fn every_warp_delta_add_overflow_is_typed() -> TestResult {
    for delta_index in 0..4 {
        let mut params = IDENTITY_WARP_PARAMS;
        params[delta_index + 2] = i32::MAX;
        let mut deltas = [0; 4];
        deltas[delta_index] = 1;
        let Err(error) = apply_warp_delta(
            params,
            WarpDeltaSyntax {
                deltas: Some(deltas),
                six_param: true,
            },
            Mv::ZERO,
            &block(),
        ) else {
            return Err("warp delta addition overflow was accepted".into());
        };
        assert!(is_warp_state(&error));
    }
    Ok(())
}

#[test]
fn rotzoom_negation_overflow_is_typed() -> TestResult {
    let mut params = IDENTITY_WARP_PARAMS;
    params[3] = i32::MIN;
    let Err(error) = apply_warp_delta(
        params,
        WarpDeltaSyntax {
            deltas: Some([0; 4]),
            six_param: false,
        },
        Mv::ZERO,
        &block(),
    ) else {
        return Err("ROTZOOM negation overflow was accepted".into());
    };
    assert!(is_warp_state(&error));
    Ok(())
}

#[test]
fn maximum_av2_warp_deltas_apply_without_error() -> TestResult {
    apply_warp_delta(
        IDENTITY_WARP_PARAMS,
        WarpDeltaSyntax {
            deltas: Some([14_336, -14_336, 14_336, -14_336]),
            six_param: true,
        },
        Mv {
            row: (1 << 16) - 1,
            col: -(1 << 16) + 1,
        },
        &block(),
    )?;
    Ok(())
}
