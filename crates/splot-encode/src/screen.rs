// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Screen-content detection: drawn content (text, UI, graphics) has few
//! distinct values in blocks that are not flat, camera content has many.

use crate::frame::SourceFrame;
use crate::kernels::kernels;

/// The side of the luma blocks the detector counts.
const BLOCK: usize = 8;
/// A block that is not flat with at most this many luma values looks drawn.
const DRAWN_COLOURS: usize = 8;
/// A luma range at most this (in 8-bit steps) is flat.
const FLAT_RANGE_8BIT: u16 = 8;
/// The share of drawn blocks among the blocks that are not flat, in 1/64,
/// that marks screen content: measured 19/64 on screendata and 35/64 on
/// desktop_credits, at most 1/64 on park_joy, paris, niklas and rush_hour.
const SCREEN_SHARE_64: usize = 6;

/// Whether `frame` looks like screen content by the luma of its whole 8x8
/// blocks.
pub(crate) fn is_screen_content(frame: &SourceFrame) -> bool {
    let [luma, ..] = frame.plane_sizes();
    let (width, height) = (luma.width(), luma.height());
    let samples = &frame.planes()[0];
    let flat = FLAT_RANGE_8BIT << (frame.bit_depth().max_sample().count_ones() - 8);
    let mut block = Vec::with_capacity(BLOCK * BLOCK);
    let (mut textured, mut drawn) = (0usize, 0usize);
    for y in (0..height / BLOCK * BLOCK).step_by(BLOCK) {
        for x in (0..width / BLOCK * BLOCK).step_by(BLOCK) {
            block.clear();
            for row in y..y + BLOCK {
                let at = row * width + x;
                block.extend_from_slice(samples.get(at..at + BLOCK).unwrap_or(&[]));
            }
            let (colours, range) = kernels().block_colours(&block, DRAWN_COLOURS);
            if range > flat {
                textured += 1;
                drawn += usize::from(colours <= DRAWN_COLOURS);
            }
        }
    }
    textured > 0 && drawn * 64 >= textured * SCREEN_SHARE_64
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::{BitDepth, PixelFormat};

    fn frame(bit_depth: BitDepth, luma: impl Fn(usize, usize) -> u16) -> SourceFrame {
        let mut config = Config::new(64, 32);
        config.bit_depth = bit_depth;
        config.chroma = PixelFormat::Yuv420;
        let y = (0..64 * 32).map(|i| luma(i % 64, i / 64)).collect();
        let chroma = || vec![128 << (bit_depth.max_sample().count_ones() - 8); 32 * 16];
        SourceFrame::new(&config, 0, [y, chroma(), chroma()]).unwrap()
    }

    #[test]
    fn text_on_a_flat_page_is_screen_content() {
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            let scale = bit_depth.max_sample() / 255;
            let text = frame(bit_depth, |x, y| {
                if (x * 7 + y * 3) % 11 < 3 { 20 * scale } else { 230 * scale }
            });
            assert!(is_screen_content(&text), "{bit_depth:?}");
        }
    }

    #[test]
    fn camera_texture_and_flat_frames_are_not() {
        let noise = frame(BitDepth::Eight, |x, y| ((x * 37 + y * 91 + x * y * 13) % 97) as u16 + 60);
        assert!(!is_screen_content(&noise));
        assert!(!is_screen_content(&frame(BitDepth::Ten, |_, _| 512)));
    }
}
