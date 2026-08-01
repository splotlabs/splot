// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Active-bit-depth range scans shared by every sample trust boundary.
//!
//! Feature tracking: `RECON-CURRENT-FRAME-WORKSPACE`.

use std::simd::{Simd, cmp::SimdOrd, num::SimdUint};

use crate::ReconSample;

/// Reports whether any sample of any storage type exceeds the active bit depth.
///
/// `u16` storage takes the lane-group scan; every other storage type keeps the
/// scalar walk its own `to_u16` conversion needs.
pub(crate) fn samples_exceed<T: ReconSample>(samples: &[T], max_sample: u16) -> bool {
    T::u16_slice(samples).map_or_else(
        || samples.iter().any(|sample| sample.to_u16() > max_sample),
        |samples| u16_samples_exceed(samples, max_sample),
    )
}

/// Reports whether any `u16` sample exceeds the active bit depth.
///
/// The scan accumulates a running lane-wise peak and compares once at the end
/// rather than comparing and reducing every lane group: a clean buffer — every
/// buffer on a conforming stream — costs one load and one `umax` per group
/// instead of a compare, a cross-lane reduction and a branch. The comparison is
/// against the same peak either way, so the answer is unchanged; only the error
/// path loses an early exit it does not need, because it already rescans sample
/// by sample to name the offender.
pub(crate) fn u16_samples_exceed(samples: &[u16], max_sample: u16) -> bool {
    const LANES: usize = 16;
    let (chunks, remainder) = samples.as_chunks::<LANES>();
    let peak = chunks
        .iter()
        .fold(Simd::<u16, LANES>::splat(0), |peak, chunk| {
            peak.simd_max(Simd::from_array(*chunk))
        });
    peak.reduce_max() > max_sample || remainder.iter().any(|&sample| sample > max_sample)
}
