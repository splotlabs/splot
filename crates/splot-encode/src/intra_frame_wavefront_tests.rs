// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Wavefront superblock search. The writer of a wavefront tile checks in
//! tests that its reconstruction of every superblock is the row facade's.

use super::*;
use crate::preset::PresetParams;

/// `preset` with the wavefront search `on`.
fn with_wavefront(preset: u8, on: bool) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        wavefront: on,
        ..*Preset::new(preset).unwrap().params()
    }))
}

/// A key frame and two inter frames of a pan, coded as `case` on `threads`
/// threads: the units and the encoder recon.
fn wavefront_stream(
    threads: usize,
    preset: &'static PresetParams,
    case: WavefrontCase,
) -> (Vec<Vec<u8>>, Vec<u8>) {
    let (preset_index, (width, height, qp), (bit_depth, chroma), (cols_log2, rows_log2), delta_q) =
        case;
    let mut config = config(width, height, bit_depth, preset_index, qp);
    config.chroma = chroma;
    config.tiles = TileLayout {
        cols_log2,
        rows_log2,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let mut coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let (mut units, mut recon) = (Vec::new(), Vec::new());
    for index in 0..3 {
        let plan = crate::gop::plan(u64::from(index), config.keyint);
        let offsets = if delta_q {
            super::delta_q::offsets(&config, index as usize)
        } else {
            Vec::new()
        };
        let mut coded = coder
            .code_frame(
                &super::tip::zooming_frame(&config, index),
                &plan,
                qp,
                &offsets,
            )
            .unwrap();
        coder.commit(&mut coded).unwrap();
        recon.extend(recon_bytes(&coded, config.bit_depth));
        units.push(coded.temporal_unit);
    }
    (units, recon)
}

/// `(preset, (width, height, qp), (bit depth, chroma), tiles, superblock
/// offsets)`.
type WavefrontCase = (u8, (u32, u32, u16), (BitDepth, PixelFormat), (u8, u8), bool);

const TEN_420: (BitDepth, PixelFormat) = (BitDepth::Ten, PixelFormat::Yuv420);

/// 64x64 superblocks in 4x3, in two tile columns of 3x3, in two tile rows
/// (the second starts below the frame top), with offsets that clip at
/// `MaxQ`, and in 4:2:2 and 4:4:4; 128x128 superblocks in 3x2 with offsets
/// and in two tile columns of 3x2 and 2x2.
const WAVEFRONT_CASES: [WavefrontCase; 8] = [
    (9, (200, 136, 120), TEN_420, (0, 0), false),
    (9, (384, 136, 160), TEN_420, (1, 0), false),
    (10, (200, 264, 150), TEN_420, (0, 1), false),
    (
        10,
        (200, 136, 240),
        (BitDepth::Eight, PixelFormat::Yuv420),
        (0, 0),
        true,
    ),
    (
        10,
        (200, 136, 130),
        (BitDepth::Ten, PixelFormat::Yuv422),
        (0, 0),
        false,
    ),
    (
        10,
        (200, 136, 130),
        (BitDepth::Eight, PixelFormat::Yuv444),
        (0, 0),
        true,
    ),
    (6, (300, 200, 140), TEN_420, (0, 0), true),
    (6, (520, 200, 140), TEN_420, (1, 0), false),
];

/// Wavefront key and inter frames, with every loop filter of the preset row,
/// decode to the encoder recon, write the same bytes on 1, 2 and 8 threads,
/// and differ from the raster-order search.
#[test]
fn wavefront_streams_decode_to_the_encoder_recon_on_any_thread_count() {
    for case in WAVEFRONT_CASES {
        let on = with_wavefront(case.0, true);
        let (one, recon) = wavefront_stream(1, on, case);
        let name = format!("{case:?}");
        assert_same_samples(&decode(&one.concat()), &recon, &name);
        for threads in [2, 8] {
            assert!(
                wavefront_stream(threads, on, case).0 == one,
                "{name}: {threads} threads"
            );
        }
        let off = wavefront_stream(2, with_wavefront(case.0, false), case).0;
        assert_ne!(off, one, "{name}: the wavefront changes no byte");
    }
}

/// AVM decodes wavefront streams like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_wavefront_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (index, case) in WAVEFRONT_CASES.into_iter().enumerate() {
        let (units, recon) = wavefront_stream(4, with_wavefront(case.0, true), case);
        let avm = avm_decode(
            &avmdec,
            &units,
            (case.1.0, case.1.1),
            case.2.0,
            &format!("wavefront{index}"),
        );
        assert!(avm == recon, "{case:?}: AVM output differs");
    }
}
