// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Lossless coding (§ 5.18.6.1 `CodedLossless`): `qp` 0 decodes to the source.

use super::*;
use crate::encoder::Encoder;
use crate::test_support::{avm_decode, avm_decoder};

/// The units and the source bytes of `frames` panning frames of `chroma` at
/// `bit_depth`, coded lossless at `preset` with the `intra_only` field and
/// `overrides` on `threads` threads with a key frame every 6 frames.
fn lossless_stream(
    (chroma, bit_depth, preset, intra_only): (PixelFormat, BitDepth, u8, u8),
    threads: usize,
    frames: u32,
    overrides: &[(&str, &str)],
) -> (Vec<Vec<u8>>, Vec<u8>) {
    let mut config = config(96, 72, bit_depth, preset, 0);
    config.chroma = chroma;
    config.keyint = core::num::NonZeroU32::new(6).unwrap();
    config.threads = splot_parallel::ThreadCount::from(threads);
    config
        .preset_overrides
        .push(("intra_only".to_owned(), intra_only.to_string()));
    for &(name, value) in overrides {
        config
            .preset_overrides
            .push((name.to_owned(), value.to_owned()));
    }
    let mut encoder = Encoder::new(config.clone()).unwrap();
    let mut source = Vec::new();
    for index in 0..frames {
        let frame = panning_frame(&config, index);
        source.extend(raw_bytes(
            frame.planes().iter().map(Vec::as_slice),
            bit_depth,
        ));
        encoder.send_frame(frame).unwrap();
    }
    encoder.flush().unwrap();
    let units = core::iter::from_fn(|| encoder.receive_packet().unwrap())
        .map(|packet| packet.data)
        .collect();
    (units, source)
}

const CASES: [(PixelFormat, BitDepth, u8, u8); 5] = [
    (PixelFormat::Yuv420, BitDepth::Eight, 6, 0),
    (PixelFormat::Yuv444, BitDepth::Ten, 6, 0),
    (PixelFormat::Yuv420, BitDepth::Ten, 10, 0),
    (PixelFormat::Yuv422, BitDepth::Eight, 0, 0),
    (PixelFormat::Yuv420, BitDepth::Eight, 10, 1),
];

/// Intra, intra-only and inter frames, with a later key frame and SDP trees,
/// decode to the source samples exactly, with the same bytes on 1, 2 and 8
/// threads.
#[test]
fn lossless_streams_decode_to_the_source_on_any_thread_count() {
    each_case(CASES, |case| {
        let name = format!("{case:?}");
        crate::tile_writer::SDP_CODED.set(0);
        let (units, source) = lossless_stream(case, 1, 8, &[]);
        assert!(
            crate::tile_writer::SDP_CODED.get() > 0,
            "{name}: no SDP tree"
        );
        assert_same_samples(&decode(&units.concat()), &source, &name);
        for threads in [2, 8] {
            assert_eq!(
                lossless_stream(case, threads, 8, &[]).0,
                units,
                "{name} {threads}"
            );
        }
    });
}

const DPCM: &[(&str, &str)] = &[("lossless_dpcm", "1")];

/// With `lossless_dpcm`, DPCM luma and chroma leaves next to SDP trees decode
/// to the source samples exactly, with the same bytes on 1, 2 and 8 threads.
#[test]
fn lossless_dpcm_streams_decode_to_the_source_on_any_thread_count() {
    each_case(CASES, |case| {
        let name = format!("{case:?}");
        crate::tile_writer::SDP_CODED.set(0);
        crate::tile_writer::DPCM_CODED.set([0; 2]);
        let (units, source) = lossless_stream(case, 1, 8, DPCM);
        let [luma, chroma] = crate::tile_writer::DPCM_CODED.get();
        assert!(luma > 0 && chroma > 0, "{name}: DPCM {luma} {chroma}");
        assert!(
            crate::tile_writer::SDP_CODED.get() > 0,
            "{name}: no SDP tree"
        );
        assert_same_samples(&decode(&units.concat()), &source, &name);
        for threads in [2, 8] {
            assert_eq!(
                lossless_stream(case, threads, 8, DPCM).0,
                units,
                "{name} {threads}"
            );
        }
    });
}

/// AVM decodes the lossless streams, with and without DPCM, to the source
/// samples.
#[test]
#[ignore = "needs SPLOT_AVM_DECODER"]
fn avm_decodes_lossless_streams_to_the_source() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for case in CASES {
        for overrides in [&[][..], DPCM] {
            let (units, source) = lossless_stream(case, 1, 8, overrides);
            let avm = avm_decode(&avmdec, &units, (96, 72), case.1, "lossless");
            assert_same_samples(&avm, &source, &format!("{case:?} {overrides:?}"));
        }
    }
}
