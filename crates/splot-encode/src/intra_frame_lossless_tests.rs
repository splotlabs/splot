// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Lossless coding (§ 5.18.6.1 `CodedLossless`): `qp` 0 decodes to the source.

use super::*;
use crate::encoder::Encoder;
use crate::test_support::{avm_decode, avm_decoder};

/// The units and the source bytes of `frames` panning frames of `chroma` at
/// `bit_depth`, coded lossless at `preset` on `threads` threads with a key
/// frame every 6 frames.
fn lossless_stream(
    (chroma, bit_depth, preset): (PixelFormat, BitDepth, u8),
    threads: usize,
    frames: u32,
) -> (Vec<Vec<u8>>, Vec<u8>) {
    let mut config = config(96, 72, bit_depth, preset, 0);
    config.chroma = chroma;
    config.keyint = core::num::NonZeroU32::new(6).unwrap();
    config.threads = splot_parallel::ThreadCount::from(threads);
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

const CASES: [(PixelFormat, BitDepth, u8); 4] = [
    (PixelFormat::Yuv420, BitDepth::Eight, 6),
    (PixelFormat::Yuv444, BitDepth::Ten, 6),
    (PixelFormat::Yuv420, BitDepth::Ten, 10),
    (PixelFormat::Yuv422, BitDepth::Eight, 0),
];

/// Intra and inter frames, with a later key frame, decode to the source
/// samples exactly, with the same bytes on 1, 2 and 8 threads.
#[test]
fn lossless_streams_decode_to_the_source_on_any_thread_count() {
    each_case(CASES, |case| {
        let name = format!("{case:?}");
        let (units, source) = lossless_stream(case, 1, 8);
        assert_same_samples(&decode(&units.concat()), &source, &name);
        for threads in [2, 8] {
            assert_eq!(
                lossless_stream(case, threads, 8).0,
                units,
                "{name} {threads}"
            );
        }
    });
}

/// AVM decodes the lossless streams to the source samples.
#[test]
#[ignore = "needs SPLOT_AVM_DECODER"]
fn avm_decodes_lossless_streams_to_the_source() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for case in CASES {
        let (units, source) = lossless_stream(case, 1, 8);
        let avm = avm_decode(&avmdec, &units, (96, 72), case.1, "lossless");
        assert_same_samples(&avm, &source, &format!("{case:?}"));
    }
}
