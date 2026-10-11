// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! IBP angular blend regression tests against offline AVM reference samples.

#![allow(clippy::unwrap_used)]

use super::*;

fn rect(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
    IntraRectBlockSize::new(log2_width, log2_height).unwrap()
}

fn blend(primary: u16, second: u16, s: u16) -> u16 {
    let v = u64::from(primary) * u64::from(s) + u64::from(second) * u64::from(128 - s);
    ((v + 64) >> 7) as u16
}

#[track_caller]
fn assert_blend(actual: u16, primary: u16, second: u16, s: u16) {
    assert_eq!(actual, blend(primary, second, s));
}

#[test]
fn ibp_blend_fires_matches_avm_enabled_set() {
    for p in [
        39u16, 45, 51, 61, 67, 73, 84, 186, 197, 203, 209, 219, 225, 231,
    ] {
        assert!(ibp_blend_fires(p), "p_angle {p} should fire IBP");
    }
    for p in [0u16, 90, 135, 180, 270] {
        assert!(!ibp_blend_fires(p), "p_angle {p} must not fire IBP");
    }
}

#[test]
fn ibp_weights_zone1_p45_match_avm_reference() {
    let size = rect(5, 5);
    let mut primary = vec![200u16; 32 * 32];
    let second = vec![50u16; 32 * 32];
    apply_ibp_dr_blend_rect(size, 45, &mut primary, &second).unwrap();
    assert_blend(primary[0], 200, 50, 64);
    assert_eq!(primary[0], 125);
    assert_blend(primary[3 * 32 + 5], 200, 50, 77);
    assert_eq!(primary[3 * 32 + 5], 140);
}

#[test]
fn ibp_weights_zone3_p203_transpose_match_avm_reference() {
    let size = rect(4, 4);
    let mut primary = vec![80u16; 16 * 16];
    let second = vec![240u16; 16 * 16];
    apply_ibp_dr_blend_rect(size, 203, &mut primary, &second).unwrap();
    assert_blend(primary[2 * 16 + 3], 80, 240, 85);
    assert_eq!(primary[2 * 16 + 3], 134);
}

#[test]
fn ibp_blend_asymmetric_primary_second_is_order_sensitive() {
    let size = rect(5, 4); // 32 wide, 16 tall -> cShift=1, rShift=0.
    let mut primary = vec![0u16; 32 * 16];
    let mut second = vec![0u16; 32 * 16];
    for r in 0..16usize {
        for c in 0..32usize {
            primary[r * 32 + c] = (10 + r * 32 + c) as u16;
            second[r * 32 + c] = (4000 - (r * 32 + c)) as u16;
        }
    }
    let primary_before = primary.clone();
    apply_ibp_dr_blend_rect(size, 67, &mut primary, &second).unwrap();
    let idx = 2; // row 0, column 2 -> c>>1=1 -> s=weights67[0][1]=108.
    assert_blend(primary[idx], primary_before[idx], second[idx], 108);
    assert_blend(primary[0], primary_before[0], second[0], 93);
}

#[test]
fn ibp_disabled_mode_is_no_op() {
    assert!(
        !ibp_blend_fires(88),
        "angle 88 -> mode_index 0 -> is_ibp_enabled[0]=false"
    );
    let size = rect(4, 4);
    let mut primary = vec![123u16; 16 * 16];
    let second = vec![45u16; 16 * 16];
    apply_ibp_dr_blend_rect(size, 88, &mut primary, &second).unwrap();
    assert!(
        primary.iter().all(|&v| v == 123),
        "disabled mode must not blend"
    );
}

#[test]
fn ibp_blend_rejects_undersized_buffers() {
    let size = rect(4, 4);
    let mut primary = vec![0u16; 16 * 16 - 1];
    let second = vec![0u16; 16 * 16];
    assert!(apply_ibp_dr_blend_rect(size, 45, &mut primary, &second).is_err());
}

/// The `u16` lane blend must match the § 7.13.2.9 per-sample formula and the
/// scalar `u8` path for every enabled angle and block shape, including
/// max-valued samples.
#[test]
fn ibp_u16_lane_blend_matches_the_scalar_blend() {
    for p_angle in (0..ZONE_3_INDEX_BASE).filter(|&p| ibp_blend_fires(p)) {
        let weights = ibp_weight_table(enabled_weight_angle(p_angle).unwrap()).unwrap();
        assert!(weights.iter().flatten().all(|&s| s <= IBP_WEIGHT_MAX));
        for log2_width in 2..=6u8 {
            for log2_height in 2..=6u8 {
                let size = rect(log2_width, log2_height);
                let (width, height) = (size.width(), size.height());
                let sample = |index: usize, seed: usize| match index % 5 {
                    0 => u16::MAX,
                    1 => 0,
                    _ => ((index * 7919 + seed) % 65536) as u16,
                };
                let primary: Vec<u16> = (0..width * height).map(|i| sample(i, 3)).collect();
                let second: Vec<u16> = (0..width * height).map(|i| sample(i + 2, 11)).collect();
                let mut lanes = primary.clone();
                apply_ibp_dr_blend_rect(size, p_angle, &mut lanes, &second).unwrap();
                for (index, &got) in lanes.iter().enumerate() {
                    let (row, column) = (index / width, index % width);
                    let row_idx = row >> (height >> 5);
                    let col_idx = column >> (width >> 5);
                    let s = if p_angle < ZONE_1_MAX {
                        weights[row_idx][col_idx]
                    } else {
                        weights[col_idx][row_idx]
                    };
                    assert_eq!(got, blend(primary[index], second[index], s));
                }

                let narrow = |samples: &[u16]| samples.iter().map(|&v| v as u8).collect::<Vec<_>>();
                let mut scalar = narrow(&primary);
                apply_ibp_dr_blend_rect(size, p_angle, &mut scalar, &narrow(&second)).unwrap();
                let mut lanes = narrow(&primary)
                    .into_iter()
                    .map(u16::from)
                    .collect::<Vec<_>>();
                let second16 = narrow(&second)
                    .into_iter()
                    .map(u16::from)
                    .collect::<Vec<_>>();
                apply_ibp_dr_blend_rect(size, p_angle, &mut lanes, &second16).unwrap();
                assert_eq!(narrow(&lanes), scalar, "p_angle {p_angle} {width}x{height}");
            }
        }
    }
}
