// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The PC-Wiener taps and classes for the [`lr_madd`] run kernels.

use super::{
    PC_WIENER_CONFIG, PC_WIENER_FILTER_TAP_RADIUS, PC_WIENER_FILTERS, PC_WIENER_FULL_CLASSES,
    PC_WIENER_PREC_BITS,
};
use crate::lr_madd::{self, LaneClass, TapPairs};

/// The § 7.20.4 taps for [`lr_madd`].
pub(super) struct PcTaps;

impl TapPairs for PcTaps {
    #[cfg(target_arch = "x86_64")]
    lr_madd::madd_avx2!(PcTaps, 7, 22);
    #[cfg(target_arch = "aarch64")]
    lr_madd::by_lane_kernel!(PcTaps, 7, 22);
    const RADIUS: usize = PC_WIENER_FILTER_TAP_RADIUS;
    const TAPS: &'static [(isize, isize)] = &PC_WIENER_CONFIG;
}

/// Every class of every § 9.8 `Pc_Wiener_Filters` set as a [`LaneClass`]:
/// taps `0..12` by pair, then tap 12 plus the `1 << PC_WIENER_PREC_BITS`
/// center unit; each narrowed to `i16`, which holds the `-208..=127` table
/// range.
pub(super) static PC_WIENER_CLASSES: std::sync::LazyLock<Vec<[LaneClass; PC_WIENER_FULL_CLASSES]>> =
    std::sync::LazyLock::new(|| {
        PC_WIENER_FILTERS
            .iter()
            .map(|set| {
                set.map(|coeffs| {
                    let mut weights = [0; 16];
                    for (weight, &coeff) in weights.iter_mut().zip(&coeffs[..12]) {
                        *weight = coeff as i16;
                    }
                    (weights, (coeffs[12] + (1 << PC_WIENER_PREC_BITS)) as i16)
                })
            })
            .collect()
    });
