// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Streams with a sequence cropping window (§ 5.4.1, § 5.18.4.4, § 7.21.2).

use super::*;
use splot_core::headers::sequence::CroppingWindow;

/// The left and top offsets fall on a chroma sample (§ 6.17.4.4); the odd
/// width and height give a chroma window of `(w + subX) >> subX` samples.
const WINDOW: CroppingWindow = CroppingWindow {
    left: 4,
    right: 5,
    top: 2,
    bottom: 3,
};

/// `recon`, whole frames of `config` in display order, cut to `window` as
/// § 7.21.2 outputs them.
fn cropped(config: &Config, recon: &[u8], window: CroppingWindow) -> Vec<u8> {
    let bytes = if config.bit_depth == BitDepth::Eight {
        1
    } else {
        2
    };
    let sizes = config.plane_sizes().unwrap();
    let frame: usize = sizes
        .iter()
        .map(|size| size.width() * size.height() * bytes)
        .sum();
    let (w, h) = (
        config.width - window.left - window.right,
        config.height - window.top - window.bottom,
    );
    let mut out = Vec::new();
    for samples in recon.chunks(frame) {
        let mut plane_start = 0;
        for (plane, size) in sizes.iter().enumerate() {
            let (sx, sy) = match plane {
                0 => (0, 0),
                _ => (
                    u32::from(config.chroma.subsampling_x()),
                    u32::from(config.chroma.subsampling_y()),
                ),
            };
            let stride = size.width() * bytes;
            let (left, top) = ((window.left >> sx) as usize, (window.top >> sy) as usize);
            let (cols, rows) = (((w + sx) >> sx) as usize, ((h + sy) >> sy) as usize);
            for row in top..top + rows {
                let start = plane_start + row * stride + left * bytes;
                out.extend_from_slice(&samples[start..start + cols * bytes]);
            }
            plane_start += stride * size.height();
        }
    }
    out
}

/// A 72x40 panning stream at preset 6 (mini-GOPs, filters on) with `window`.
fn crop_stream(
    chroma: PixelFormat,
    bit_depth: BitDepth,
    window: Option<CroppingWindow>,
    threads: usize,
) -> (Config, Vec<Vec<u8>>, Vec<u8>) {
    let mut config = config(72, 40, bit_depth, 6, 150);
    config.chroma = chroma;
    config.crop = window;
    config.threads = splot_parallel::ThreadCount::from(threads);
    let (units, expected) =
        encode_frames(&config, (0..7).map(|index| panning_frame(&config, index)));
    (config, units, expected)
}

/// The 4:2:0 8-bit and 4:2:2 10-bit streams decode to the window of the
/// encoder recon, with the same bytes on 1, 2 and 8 threads; the window
/// changes only the sequence header.
#[test]
fn cropped_streams_decode_to_the_window_of_the_recon() {
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv422, BitDepth::Ten),
    ] {
        let case = format!("{chroma:?} {bit_depth:?}");
        let (config, units, recon) = crop_stream(chroma, bit_depth, Some(WINDOW), 1);
        let expected = cropped(&config, &recon, WINDOW);
        assert_same_samples(&decode(&units.concat()), &expected, &case);
        {
            let threads = 2;
            let other = crop_stream(chroma, bit_depth, Some(WINDOW), threads).1;
            assert!(other == units, "{case}: {threads} threads");
        }
        let (_, plain, plain_recon) = crop_stream(chroma, bit_depth, None, 1);
        assert_eq!(plain_recon, recon, "{case}: the window changes the recon");
        assert_eq!(plain[1..], units[1..], "{case}: the window changes a frame");
    }
}

/// A window that leaves no visible sample, one whose left or top offset
/// splits a chroma sample (§ 6.17.4.4), or one larger than the coded frame
/// after the resize, is refused at configuration; 4:4:4 takes odd offsets.
#[test]
fn windows_that_do_not_fit_the_coded_frame_are_refused() {
    let mut config = config(72, 40, BitDepth::Eight, 6, 150);
    for window in [
        CroppingWindow {
            left: 36,
            right: 36,
            ..WINDOW
        },
        CroppingWindow {
            bottom: 40,
            top: 0,
            ..WINDOW
        },
        CroppingWindow { left: 3, ..WINDOW },
    ] {
        config.crop = Some(window);
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidCrop { width: 72, .. })
        ));
    }
    config.chroma = PixelFormat::Yuv444;
    assert!(config.validate().is_ok());
    config.chroma = PixelFormat::Yuv420;
    config.crop = Some(CroppingWindow { left: 40, ..WINDOW });
    assert!(config.validate().is_ok());
    config.resize_denominator = Some(16);
    assert!(matches!(
        config.validate(),
        Err(Error::InvalidCrop { width: 36, .. })
    ));
    assert!(config.first_pass_config().unwrap().crop.is_none());
}

/// AVM decodes cropped streams to the same window as splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_cropped_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv422, BitDepth::Ten),
    ] {
        let (_, units, _) = crop_stream(chroma, bit_depth, Some(WINDOW), 1);
        let splot = decode(&units.concat());
        let name = format!("crop{chroma:?}{bit_depth:?}");
        let avm = avm_decode(&avmdec, &units, (72, 40), bit_depth, &name);
        assert!(avm == splot, "{name}: AVM output differs");
    }
}
