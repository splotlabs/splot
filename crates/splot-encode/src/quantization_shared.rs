// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared encoder quantizer helpers for the 4x4 ([`crate::quantization`]) and
//! 16x16 ([`crate::quantization_16x16`]) per-coefficient quantizers.
//!
//! Both block-size modules need the identical dequant visible-range bound and a
//! zero [`QuantizerDeltas`] constant, so they live here once instead of being
//! duplicated. This is encoder-policy arithmetic only; it emits no AV2 syntax and
//! delegates nothing to the decoder path.

use splot_recon::{BitDepth as ReconBitDepth, QuantizerDeltas};

/// Inclusive `(min, max)` range a pre-quantization coefficient may occupy for the
/// given reconstruction bit depth, matching the decoder's dequant visible range.
pub(crate) fn dequant_visible_range(bit_depth: ReconBitDepth) -> (i32, i32) {
    let bound = 1i32 << (7 + u32::from(bit_depth.bits()));
    (-bound, bound - 1)
}

/// All-zero [`QuantizerDeltas`] used when the encoder applies no per-plane DC/AC
/// quantizer offsets.
pub(crate) const fn zero_deltas() -> QuantizerDeltas {
    QuantizerDeltas {
        y_dc: 0,
        u_dc: 0,
        v_dc: 0,
        u_ac: 0,
        v_ac: 0,
    }
}
