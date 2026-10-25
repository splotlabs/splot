// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The sequence's `cfl_ds_filter_index` (§ 6.4.5): the luma down-sampling of
//! CfL prediction (§ 7.13.5) and of the chroma Wiener-NS luma taps (§ 7.20.3).

use splot_decode::encoder_syntax::{CFL_FILTERS_420, CFL_FILTERS_422};

use crate::frame::SourceFrame;
use crate::kernels::kernels;

/// The side of the chroma blocks that each get one linear fit.
const BLOCK: usize = 8;

/// The `cfl_ds_filter_index` in `0..=2` whose CfL down-sampled luma of
/// `frame` best predicts its chroma: the lowest sum, over the whole 8x8
/// chroma blocks of U and V, of the residual energy of a least-squares fit
/// `chroma = alpha * luma + beta`, the model that CfL codes. `0` on a tie and
/// without horizontal subsampling.
pub(crate) fn cfl_ds_filter(frame: &SourceFrame) -> u8 {
    let [luma, chroma, _] = frame.plane_sizes();
    let (width, height) = (chroma.width(), chroma.height());
    if width == 0 || luma.width() == width {
        return 0;
    }
    let vertical = luma.height() != height;
    let planes = frame.planes();
    let row = |y: usize| {
        let y = y.min(luma.height().saturating_sub(1));
        planes[0]
            .get(y * luma.width()..(y + 1) * luma.width())
            .unwrap_or(&[])
    };
    let mut down = vec![0; width * height];
    let mut best = (u128::MAX, 0);
    for index in 0..3 {
        let weights = if vertical {
            CFL_FILTERS_420[index]
        } else {
            [[0; 3], CFL_FILTERS_422[index], [0; 3]]
        };
        for (y, out) in down.chunks_exact_mut(width).enumerate() {
            let at = if vertical { 2 * y } else { y };
            let rows = [row(at.saturating_sub(1)), row(at), row(at + 1)];
            kernels().cfl_downsample(&weights, rows, out);
        }
        let cost = fit_residual(&down, &planes[1], width, height)
            .saturating_add(fit_residual(&down, &planes[2], width, height));
        if cost < best.0 {
            best = (cost, index as u8);
        }
    }
    best.1
}

/// `n²` times the least-squares residual energy of `chroma` on `luma`,
/// summed over the whole `BLOCK x BLOCK` blocks of two `width x height`
/// planes, where `n = BLOCK²`.
fn fit_residual(luma: &[i32], chroma: &[u16], width: usize, height: usize) -> u128 {
    let n = (BLOCK * BLOCK) as i128;
    let (mut l, mut c) = (Vec::with_capacity(BLOCK * BLOCK), Vec::new());
    let mut total = 0u128;
    for y in (0..height / BLOCK * BLOCK).step_by(BLOCK) {
        for x in (0..width / BLOCK * BLOCK).step_by(BLOCK) {
            l.clear();
            c.clear();
            for row in y..y + BLOCK {
                let at = row * width + x;
                let (Some(ls), Some(cs)) = (luma.get(at..at + BLOCK), chroma.get(at..at + BLOCK))
                else {
                    return total;
                };
                l.extend_from_slice(ls);
                c.extend(cs.iter().map(|&s| i32::from(s)));
            }
            let [ll, cc, lc] = kernels().cctx_stats(&l, &c).map(i128::from);
            let sum = |v: &[i32]| v.iter().map(|&s| i128::from(s)).sum::<i128>();
            let (sl, sc) = (sum(&l), sum(&c));
            let var_l = n * ll - sl * sl;
            let var_c = n * cc - sc * sc;
            let cov = n * lc - sl * sc;
            let explained = if var_l > 0 { cov * cov / var_l } else { 0 };
            total = total.saturating_add(u128::try_from(var_c - explained).unwrap_or(0));
        }
    }
    total
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use splot_core::annexb::parse_annex_b_obus_partial;
    use splot_core::bitio::BitReader;
    use splot_core::headers::sequence::parse_sequence_header;
    use splot_core::span::ByteOffset;
    use splot_core::types::ObuType;

    use super::*;
    use crate::config::{Config, RateControlMode};
    use crate::preset::Preset;
    use crate::{BitDepth, Encoder, PixelFormat};

    /// A 64x48 frame of `chroma` whose U and V are its luma down-sampled by
    /// CfL filter `index`, scaled and offset per plane, over a luma texture
    /// that differs between neighbouring columns and rows.
    fn sited_frame(chroma: PixelFormat, index: usize) -> SourceFrame {
        let mut config = Config::new(64, 48);
        config.bit_depth = BitDepth::Ten;
        config.chroma = chroma;
        let luma: Vec<u16> = (0..64 * 48)
            .map(|i| ((i % 64) * 37 + (i / 64) * 91 + (i % 7) * (i / 64 % 5) * 29) as u16 % 1024)
            .collect();
        let [_, size, _] = config.plane_sizes().unwrap();
        let sample = |x: usize, y: usize| i32::from(luma[y.min(47) * 64 + x.min(63)]);
        let down: Vec<i32> = (0..size.width() * size.height())
            .map(|i| {
                let (x, y) = (i % size.width(), i / size.width());
                let (lx, ly) = (2 * x, if size.height() == 24 { 2 * y } else { y });
                let mut t = 0;
                for (dy, taps) in [-1isize, 0, 1].into_iter().zip(match size.height() {
                    24 => CFL_FILTERS_420[index],
                    _ => [[0; 3], CFL_FILTERS_422[index], [0; 3]],
                }) {
                    for (dx, w) in [-1isize, 0, 1].into_iter().zip(taps) {
                        let at = |v: usize, d: isize| v.saturating_add_signed(d);
                        t += w * sample(at(lx, dx), at(ly, dy));
                    }
                }
                t / 8
            })
            .collect();
        let u = down.iter().map(|&s| (s / 2 + 100) as u16).collect();
        let v = down.iter().map(|&s| (900 - s * 3 / 4) as u16).collect();
        SourceFrame::new(&config, 0, [luma, u, v]).unwrap()
    }

    #[test]
    fn picks_the_filter_that_made_the_chroma() {
        for chroma in [PixelFormat::Yuv420, PixelFormat::Yuv422] {
            for index in 0..3 {
                let frame = sited_frame(chroma, index);
                assert_eq!(usize::from(cfl_ds_filter(&frame)), index, "{chroma:?}");
            }
        }
    }

    /// The encoder codes the index of its first frame in the sequence header.
    #[test]
    fn the_encoder_codes_the_index_of_the_first_frame() {
        let frame = sited_frame(PixelFormat::Yuv420, 2);
        let mut config = Config::new(64, 48);
        config.bit_depth = BitDepth::Ten;
        config.preset = Preset::new(10).unwrap();
        config.rate_control = RateControlMode::ConstantQp { qp: 120 };
        let mut encoder = Encoder::new(config).unwrap();
        encoder.send_frame(frame).unwrap();
        encoder.flush().unwrap();
        let packet = encoder.receive_packet().unwrap().unwrap();
        let parsed = parse_annex_b_obus_partial(&packet.data);
        let obu = parsed
            .obus
            .iter()
            .find(|obu| obu.header.obu_type == ObuType::SequenceHeader)
            .unwrap();
        let seq = parse_sequence_header(&mut BitReader::new(obu.payload, ByteOffset::new(0)));
        assert_eq!(seq.unwrap().intra.unwrap().cfl_ds_filter_index, 2);
    }

    #[test]
    fn keeps_zero_without_subsampling() {
        let mut config = Config::new(16, 16);
        config.chroma = PixelFormat::Yuv444;
        let plane = || (0..256).map(|i| (i * 13 % 251) as u16).collect();
        let frame = SourceFrame::new(&config, 0, [plane(), plane(), plane()]).unwrap();
        assert_eq!(cfl_ds_filter(&frame), 0);
    }
}
