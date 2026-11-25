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
    let tables = [0, 1, 2, 3].map(|i| u8x64::load_array_ref(simd, &quarters[i]));
    let (rows, tail) = row.as_chunks_mut::<64>();
    let (noise, tail_noise) = noise.as_chunks::<64>();
    for (row, noise) in rows.iter_mut().zip(noise) {
        apply_block(simd, row, noise, tables, params);
    }
    if !tail.is_empty() {
        let len = tail.len();
        let mut row = [0; 64];
        let mut noise = [0; 64];
        row[..len].copy_from_slice(tail);
        noise[..len].copy_from_slice(tail_noise);
        apply_block(simd, &mut row, &noise, tables, params);
        tail.copy_from_slice(&row[..len]);
    }
}

#[expect(
    clippy::inline_always,
    reason = "keep each block in the AVX-512 row scope"
)]
#[inline(always)]
fn apply_block<S: Simd>(
    simd: S,
    row: &mut [u8; 64],
    noise: &[i16; 64],
    [a, b, c, d]: [u8x64<S>; 4],
    params: NoiseParams,
) {
    let samples = u8x64::load_array_ref(simd, row);
    let index_in_half = samples & u8x64::splat(simd, 127);
    let scale = samples.simd_ge(u8x64::splat(simd, 128)).select(
        simd.concat_swizzle_dyn_u8x64(c, d, index_in_half),
        simd.concat_swizzle_dyn_u8x64(a, b, index_in_half),
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
}
