// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn zero_mv_luma_is_full_pel_origin() {
    let s = derive_plane_scaling(0, 0, 0, 0, 0, 0, 16, 16, 64, 64);
    assert_eq!(s.step_x, 1024);
    assert_eq!(s.step_y, 1024);
    assert_eq!(s.last_x, 63);
    assert_eq!(s.last_y, 63);
    assert_eq!((s.start_x >> 6) & 15, 0);
    assert_eq!((s.start_y >> 6) & 15, 0);
}

#[test]
fn fractional_mv_produces_subpel_phase() {
    let s = derive_plane_scaling(0, 0, 0, 4, 0, 0, 16, 16, 64, 64);
    assert_ne!((s.start_x >> 6) & 15, 0, "horizontal sub-pel phase set");
    assert_eq!((s.start_y >> 6) & 15, 0, "vertical phase zero");
}

#[test]
fn chroma_420_halves_dimensions() {
    let s = derive_plane_scaling(0, 0, 0, 0, 1, 1, 16, 16, 32, 32);
    assert_eq!(s.last_x, 31);
    assert_eq!(s.last_y, 31);
}
