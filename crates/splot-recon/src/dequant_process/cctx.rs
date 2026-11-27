// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.14.6 get dequant process of a cross-chroma transform pair: the
//! § 7.14.4 dequantization of the U and V blocks and their rotation, in one
//! pass over the coefficients.

use fearless_simd::{Simd, SimdBase as _, i32x8};
use splot_core::simd::SimdBackend;

use super::{DequantBlockParams, dequant_block_prefix, dequant_lanes, group_quantizers};
use crate::{ReconError, Result};

/// AV2 § 3 `CCTX_PREC_BITS`: the fraction bits of `Cctx_Mtx`.
const CCTX_PREC_BITS: u32 = 8;

/// AV2 § 7.14.6 `Cctx_Mtx`: `[cos, sin] << CCTX_PREC_BITS` of the rotation
/// that `cctxType - 1` selects.
pub const CCTX_MTX: [[i32; 2]; 6] = [
    [181, 181],
    [222, 128],
    [128, 222],
    [181, -181],
    [222, -128],
    [128, -222],
];

/// AV2 § 7.14.6 dequantized coefficients of a cross-chroma pair: the § 7.14.4
/// `Dequant` blocks of U and V (`params`, `quant` and `out` in that order),
/// rotated by `rotation`, the pair's [`CCTX_MTX`] row.
///
/// Each `quant` may stop early as in [`dequantize_block`](super::dequantize_block).
/// The pair is written over the whole rows that the longer `quant` reaches,
/// the shorter one counting as zero there; the returned length is that prefix
/// of both `out` blocks, and the rest of them is left untouched.
///
/// # Errors
/// Returns the errors of [`dequantize_block`](super::dequantize_block) for
/// either block, and [`ReconError::InvalidDequantBlockShape`] if the V block
/// does not have the shape and bit depth of the U block.
pub fn dequantize_cctx_pair_with<B: SimdBackend>(
    backend: B,
    params: [&DequantBlockParams; 2],
    quant: [&[i32]; 2],
    rotation: [i32; 2],
    [u_out, v_out]: [&mut [i32]; 2],
) -> Result<usize> {
    let [u, v] = params;
    if (v.tx_width, v.tx_height, v.bit_depth) != (u.tx_width, u.tx_height, u.bit_depth) {
        return Err(ReconError::InvalidDequantBlockShape {
            tx_width: v.tx_width,
            tx_height: v.tx_height,
        });
    }
    let u_coded = dequant_block_prefix(u, quant[0].len(), u_out.len())?;
    let v_coded = dequant_block_prefix(v, quant[1].len(), v_out.len())?;
    let coded = u_coded.max(v_coded);
    let out = [&mut u_out[..coded], &mut v_out[..coded]];
    let narrow = backend.narrow();
    splot_core::with_simd!(narrow, simd => pair_groups(simd, params, quant, rotation, out))?;
    Ok(coded)
}

/// Dequantizes and rotates the pair into the as long `u_out` and `v_out`,
/// eight coefficients a group. A group past the end of a `quant` is zero, and
/// one that it ends in loads zero-padded. Whole rows of 4, 8, 16 or 32 end on
/// a group or on half of one, which stores its low four lanes.
#[inline]
fn pair_groups<S: Simd>(
    simd: S,
    [u_params, v_params]: [&DequantBlockParams; 2],
    [u_quant, v_quant]: [&[i32]; 2],
    [cos, sin]: [i32; 2],
    [u_out, v_out]: [&mut [i32]; 2],
) -> Result<()> {
    let bound = 1i32 << (7 + u32::from(u_params.bit_depth.bits()));
    let dequant = |params: &DequantBlockParams, quant: &[i32], group: usize| {
        let coded = &quant[quant.len().min(8 * group)..];
        let (lanes, len) = match coded.first_chunk() {
            Some(lanes) => (*lanes, 8),
            None if coded.is_empty() => return Ok(i32x8::splat(simd, 0)),
            None => {
                let mut lanes = [0i32; 8];
                lanes[..coded.len()].copy_from_slice(coded); // splot-copy-ok: zero-padded lane group
                (lanes, coded.len())
            }
        };
        let q2 = group_quantizers(simd, params, group, len)?;
        let denom_shift = params.dq_denom.max(1).trailing_zeros();
        let lanes = i32x8::load_array(simd, lanes);
        Ok(dequant_lanes(simd, lanes, q2, denom_shift, bound))
    };
    // `Round2Signed(sum, CCTX_PREC_BITS)`: a negative sum rounds its magnitude.
    let round = |sum: i32x8<S>| {
        (sum + (sum >> (i32::BITS - 1)) + (1 << (CCTX_PREC_BITS - 1))) >> CCTX_PREC_BITS
    };
    let rotated = |group: usize| -> Result<[i32x8<S>; 2]> {
        let u = dequant(u_params, u_quant, group)?;
        let v = dequant(v_params, v_quant, group)?;
        Ok([u * cos - v * sin, u * sin + v * cos].map(|sum| round(sum).clamp(-bound, bound - 1)))
    };
    let (u_groups, u_half) = u_out.as_chunks_mut::<8>();
    let (v_groups, v_half) = v_out.as_chunks_mut::<8>();
    for (group, (u, v)) in u_groups.iter_mut().zip(v_groups.iter_mut()).enumerate() {
        let [next_u, next_v] = rotated(group)?;
        *u = next_u.into();
        *v = next_v.into();
    }
    if let (Ok(u), Ok(v)) = (
        <&mut [i32; 4]>::try_from(u_half),
        <&mut [i32; 4]>::try_from(v_half),
    ) {
        let [next_u, next_v] = rotated(u_groups.len())?;
        *u = simd.split_i32x8(next_u).0.into();
        *v = simd.split_i32x8(next_v).0.into();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::reference_dequantize;
    use super::super::{QmDequant, dequantize_block};
    use super::*;
    use crate::BitDepth;
    use crate::math::round2_signed_i32;
    use splot_core::simd::{SimdPolicy, SimdScope, resolve_backend};

    /// The scalar § 7.14.6 rotation of one dequantized pair: the oracle of
    /// [`pair_groups`].
    fn rotate_pair(u: i32, v: i32, [cos, sin]: [i32; 2], bit_depth: BitDepth) -> [i32; 2] {
        let bound = 1i32 << (u32::from(bit_depth.bits()) + 7);
        [u * cos - v * sin, u * sin + v * cos]
            .map(|sum| round2_signed_i32(sum, CCTX_PREC_BITS).clamp(-bound, bound - 1))
    }

    struct Pair<'a> {
        params: [&'a DequantBlockParams; 2],
        quant: [&'a [i32]; 2],
        rotation: [i32; 2],
        out: [&'a mut [i32]; 2],
    }

    impl SimdScope<Result<usize>> for Pair<'_> {
        fn run<B: SimdBackend>(self, backend: B) -> Result<usize> {
            dequantize_cctx_pair_with(backend, self.params, self.quant, self.rotation, self.out)
        }
    }

    fn pair<'a>(
        params: [&'a DequantBlockParams; 2],
        quant: [&'a [i32]; 2],
        rotation: [i32; 2],
        out: [&'a mut [i32]; 2],
    ) -> Result<usize> {
        resolve_backend(SimdPolicy::Auto).unwrap().enter(Pair {
            params,
            quant,
            rotation,
            out,
        })
    }

    /// The lane groups must reproduce the per-coefficient oracle exactly for
    /// every `Cctx_Mtx` row, block shape and bit depth, flat and under a
    /// quantization matrix, over randomized coefficients including the `i32`
    /// extremes and quantizers that drive both planes to the clamp bounds, for
    /// equal and unequal coded lengths; the rows past the pair stay untouched.
    #[test]
    fn pair_groups_match_the_per_coefficient_reference() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            for (tx_width, tx_height) in [(4, 4), (4, 16), (8, 4), (8, 8), (16, 8), (32, 32)] {
                let samples = tx_width * tx_height;
                for (case, &rotation) in CCTX_MTX.iter().cycle().take(24).enumerate() {
                    let mut plane = |chroma_matrix: bool| {
                        let mut quantizer = || {
                            if case % 4 == 3 {
                                u32::MAX - (next() % 9) as u32
                            } else {
                                (next() % 8192) as u32 + 1
                            }
                        };
                        let params = DequantBlockParams {
                            dc_quant: quantizer(),
                            ac_quant: quantizer(),
                            tx_width,
                            tx_height,
                            dq_denom: 1 << (case % 3),
                            bit_depth,
                            qm: (case % 2 == 1).then_some(QmDequant {
                                seg_level: case % 15,
                                plane_is_chroma: chroma_matrix,
                                qm_offset: 0,
                                user: None,
                            }),
                        };
                        let quant: Vec<i32> = (0..samples)
                            .map(|index| match index % 16 {
                                0 => i32::MIN,
                                1 => i32::MAX,
                                2 => 0,
                                _ => (next() as i32) >> (next() % 24),
                            })
                            .collect();
                        (params, quant)
                    };
                    let ((u_params, u_quant), (v_params, v_quant)) = (plane(false), plane(true));
                    let lengths = [0, 1, 3, 4, 7, 9, samples - 5, samples];
                    for (u_len, v_len) in lengths.iter().flat_map(|&u| lengths.map(|v| (u, v))) {
                        let coded = u_len.max(v_len).next_multiple_of(tx_width);
                        let dequant = |params, quant: &[i32]| {
                            let mut dequant = reference_dequantize(params, quant).unwrap();
                            dequant.resize(coded, 0);
                            dequant
                        };
                        let u_dequant = dequant(&u_params, &u_quant[..u_len]);
                        let v_dequant = dequant(&v_params, &v_quant[..v_len]);
                        let (mut want_u, mut want_v) =
                            (vec![i32::MAX; samples], vec![i32::MIN; samples]);
                        for (index, (&u, &v)) in u_dequant.iter().zip(&v_dequant).enumerate() {
                            [want_u[index], want_v[index]] = rotate_pair(u, v, rotation, bit_depth);
                        }

                        let (mut u_out, mut v_out) =
                            (vec![i32::MAX; samples], vec![i32::MIN; samples]);
                        let written = pair(
                            [&u_params, &v_params],
                            [&u_quant[..u_len], &v_quant[..v_len]],
                            rotation,
                            [&mut u_out, &mut v_out],
                        )
                        .unwrap();
                        let label = format!(
                            "{tx_width}x{tx_height} {bit_depth:?} case {case} lengths {u_len}/{v_len}"
                        );
                        assert_eq!(written, coded, "{label}");
                        assert_eq!((u_out, v_out), (want_u, want_v), "{label}");
                    }
                }
            }
        }
    }

    /// A plane at a clamp bound rotates to the clipped § 7.14.6 value.
    #[test]
    fn pair_at_the_clamp_bounds_is_clipped() {
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            let bound = 1i32 << (u32::from(bit_depth.bits()) + 7);
            let params = DequantBlockParams {
                bit_depth,
                ..identity_params()
            };
            let (low, high) = ([-bound - 5; 16], [bound + 5; 16]);
            let mut dequant = [0; 16];
            dequantize_block(&params, &low, &mut dequant).unwrap();
            assert_eq!(dequant, [-bound; 16]);
            for &rotation in &CCTX_MTX {
                for (u_quant, v_quant) in [(low, low), (low, high), (high, low), (high, high)] {
                    let [u, v] =
                        [u_quant[0], v_quant[0]].map(|quant| quant.clamp(-bound, bound - 1));
                    let (mut u_out, mut v_out) = ([0; 16], [0; 16]);
                    pair(
                        [&params, &params],
                        [&u_quant, &v_quant],
                        rotation,
                        [&mut u_out, &mut v_out],
                    )
                    .unwrap();
                    let [want_u, want_v] = rotate_pair(u, v, rotation, bit_depth);
                    assert_eq!((u_out, v_out), ([want_u; 16], [want_v; 16]));
                    assert!(
                        [want_u, want_v]
                            .iter()
                            .any(|&c| c == -bound || c == bound - 1)
                    );
                }
            }
        }
    }

    /// Quantizer 8 dequantizes a coefficient to itself.
    fn identity_params() -> DequantBlockParams {
        DequantBlockParams {
            dc_quant: 8,
            ac_quant: 8,
            tx_width: 4,
            tx_height: 4,
            dq_denom: 1,
            bit_depth: BitDepth::Eight,
            qm: None,
        }
    }

    #[test]
    fn minus_30_degrees_rotates_a_u_only_pair() {
        let u_quant = [-108, -54, 0, 0, 0, 0, 0, 0, 54, 0, 0, 0, 54];
        let (mut u, mut v) = ([7; 16], [7; 16]);
        let params = identity_params();
        let written = pair(
            [&params, &params],
            [&u_quant, &[]],
            CCTX_MTX[4],
            [&mut u, &mut v],
        )
        .unwrap();

        assert_eq!(written, 16);
        assert_eq!(u, [-94, -47, 0, 0, 0, 0, 0, 0, 47, 0, 0, 0, 47, 0, 0, 0]);
        assert_eq!(v, [54, 27, 0, 0, 0, 0, 0, 0, -27, 0, 0, 0, -27, 0, 0, 0]);
    }

    #[test]
    fn mismatched_pair_blocks_are_rejected() {
        let u = identity_params();
        let (mut u_out, mut v_out) = ([0; 16], [0; 32]);
        for v in [
            DequantBlockParams {
                tx_height: 8,
                ..identity_params()
            },
            DequantBlockParams {
                bit_depth: BitDepth::Ten,
                ..identity_params()
            },
        ] {
            let v_out = &mut v_out[..v.tx_width * v.tx_height];
            assert!(matches!(
                pair([&u, &v], [&[1], &[1]], CCTX_MTX[0], [&mut u_out, v_out]),
                Err(ReconError::InvalidDequantBlockShape { tx_width: 4, .. })
            ));
        }
        assert!(matches!(
            pair(
                [&u, &u],
                [&[1], &[1; 17]],
                CCTX_MTX[0],
                [&mut u_out, &mut v_out[..16]]
            ),
            Err(ReconError::DequantBlockLengthMismatch {
                expected: 16,
                quant_len: 17,
                out_len: 16
            })
        ));
        assert_eq!((u_out, v_out), ([0; 16], [0; 32]));
    }
}
