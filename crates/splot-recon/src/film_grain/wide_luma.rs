// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use fearless_simd::{Simd, i16x32, prelude::*, u8x64, u16x32};

use super::{NoiseParams, add_noise_vector};

#[expect(
    clippy::inline_always,
    reason = "keep the complete row in its SIMD scope"
)]
#[inline(always)]
pub(super) fn add_luma_noise<S: Simd>(
    simd: S,
    row: &mut [u8],
    table: &[u8; 256],
    noise: &[i16],
    params: NoiseParams,
) {
    let params = NoiseParams {
        narrow: true,
        ..params
    };
    let quarters = table.as_chunks::<64>().0;
    let [a, b, c, d] = [0, 1, 2, 3].map(|i| u8x64::load_array_ref(simd, &quarters[i]));
    let split = u8x64::splat(simd, 128);
    let apply = |row: &mut [u8; 64], noise: &[i16; 64]| {
        let samples = u8x64::load_array_ref(simd, row);
        // Keep both permutations in range. The high bit selects their half.
        let index = samples & u8x64::splat(simd, 127);
        let scale = samples.simd_ge(split).select(
            simd.concat_swizzle_dyn_u8x64(c, d, index),
            simd.concat_swizzle_dyn_u8x64(a, b, index),
        );
        let (lo, hi) = samples.widen();
        let (scale_lo, scale_hi) = scale.widen();
        let noise = noise.as_chunks::<32>().0;
        let lo = add_noise_vector(
            simd,
            lo.bitcast(),
            scale_lo.bitcast(),
            i16x32::load_array_ref(simd, &noise[0]),
            params,
        );
        let hi = add_noise_vector(
            simd,
            hi.bitcast(),
            scale_hi.bitcast(),
            i16x32::load_array_ref(simd, &noise[1]),
            params,
        );
        lo.bitcast::<u16x32<S>>()
            .narrow(hi.bitcast::<u16x32<S>>())
            .store_array(row);
    };
    let (rows, tail) = row.as_chunks_mut::<64>();
    let (noise, tail_noise) = noise.as_chunks::<64>();
    for (row, noise) in rows.iter_mut().zip(noise) {
        apply(row, noise);
    }
    if !tail.is_empty() {
        let len = tail.len();
        let mut row = [0; 64];
        let mut noise = [0; 64];
        row[..len].copy_from_slice(tail);
        noise[..len].copy_from_slice(tail_noise);
        apply(&mut row, &noise);
        tail.copy_from_slice(&row[..len]);
    }
}
