// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 §7.13.2.9 IBP angular blend tests. The reference weights and blended
//! samples are computed offline from the verbatim AVM `av2_dr_prediction_z1_info`
//! / `av2_highbd_ibp_dr_prediction_z1_c` / `_z3_c` algorithms, so a regression in
//! the weight derivation, the cShift/rShift indexing, or the Round2 blend changes
//! a pinned sample.

#![allow(clippy::unwrap_used)]

use super::*;

fn rect(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
    IntraRectBlockSize::new(log2_width, log2_height).unwrap()
}

/// AVM Round2(primary*s + second*(128-s), 7) reference.
fn blend(primary: u16, second: u16, s: u16) -> u16 {
    let v = u64::from(primary) * u64::from(s) + u64::from(second) * u64::from(128 - s);
    ((v + 64) >> 7) as u16
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
    assert_eq!(
        primary[0],
        blend(200, 50, 64),
        "(0,0) must use weights45[0][0]=64"
    );
    assert_eq!(primary[0], 125);
    assert_eq!(
        primary[3 * 32 + 5],
        blend(200, 50, 77),
        "(3,5) must use weights45[1][2]=77"
    );
    assert_eq!(primary[3 * 32 + 5], 140);
}

#[test]
fn ibp_weights_zone3_p203_transpose_match_avm_reference() {
    let size = rect(4, 4);
    let mut primary = vec![80u16; 16 * 16];
    let second = vec![240u16; 16 * 16];
    apply_ibp_dr_blend_rect(size, 203, &mut primary, &second).unwrap();
    assert_eq!(
        primary[2 * 16 + 3],
        blend(80, 240, 85),
        "zone-3 p203 (weight angle 67) transposes the lookup: s=weights67[3][2]=85"
    );
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
    assert_eq!(
        primary[idx],
        blend(primary_before[idx], second[idx], 108),
        "asymmetric blend at (0,2) must use weights67[0][1]=108"
    );
    assert_eq!(primary[0], blend(primary_before[0], second[0], 93));
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
