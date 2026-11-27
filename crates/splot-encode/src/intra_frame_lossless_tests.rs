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
    case: (PixelFormat, BitDepth, u8, u8),
    threads: usize,
    frames: u32,
    overrides: &[(&str, &str)],
) -> (Vec<Vec<u8>>, Vec<u8>) {
    lossless_stream_of(panning_frame, case, threads, frames, overrides)
}

/// [`lossless_stream`] of the frames that `content` makes.
fn lossless_stream_of(
    content: fn(&Config, u32) -> SourceFrame,
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
        let frame = content(&config, index);
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
        {
            let threads = 2;
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
        {
            let threads = 2;
            assert_eq!(
                lossless_stream(case, threads, 8, DPCM).0,
                units,
                "{name} {threads}"
            );
        }
    });
}

const INTER_TX: [&[(&str, &str)]; 2] = [
    &[("lossless_inter_tx", "1")],
    &[("lossless_inter_tx", "2"), ("lossless_dpcm", "1")],
];

/// With `lossless_inter_tx`, inter leaves code IDTX 4x4 luma units (1) and
/// `lossless_tx_size = 1` leaves (2, next to DPCM), whose chroma takes IDTX
/// by each § 5.20.7.29 rule; the streams decode to the source samples with
/// the same bytes on 1, 2 and 8 threads.
#[test]
fn lossless_inter_tx_streams_decode_to_the_source_on_any_thread_count() {
    each_case(CASES, |case| {
        for overrides in INTER_TX {
            let name = format!("{case:?} {overrides:?}");
            crate::tile_writer::LOSSLESS_IDTX_CODED.set([0; 5]);
            crate::tile_writer::SDP_CODED.set(0);
            let (units, source) = lossless_stream(case, 1, 8, overrides);
            let coded = crate::tile_writer::LOSSLESS_IDTX_CODED.get();
            if case.3 == 0 {
                assert!(coded[0] > 0, "{name}: no IDTX 4x4 unit {coded:?}");
                if overrides.len() > 1 {
                    assert!(coded[1] > 0 && coded[2] > 0, "{name}: {coded:?}");
                } else {
                    assert!(coded[3] > 0, "{name}: no collocated chroma {coded:?}");
                }
            }
            assert!(
                crate::tile_writer::SDP_CODED.get() > 0,
                "{name}: no SDP tree"
            );
            assert_same_samples(&decode(&units.concat()), &source, &name);
            for threads in [2, 8] {
                let other = lossless_stream(case, threads, 8, overrides).0;
                assert_eq!(other, units, "{name} {threads}");
            }
        }
    });
}

/// The three § 5.20.7.29 chroma rules of lossless inter leaves code IDTX
/// chroma that the decoder reconstructs: of a `lossless_tx_size = 1` leaf,
/// of the collocated luma unit, and in 4:2:0 of the leaf's first luma unit
/// when the chroma reference is another leaf (4xN and Nx4 leaves at p0).
#[test]
fn lossless_inter_chroma_follows_each_luma_rule() {
    let case = (PixelFormat::Yuv420, BitDepth::Eight, 0, 0);
    for (overrides, slot) in [(INTER_TX[0], 4), (INTER_TX[1], 2)] {
        crate::tile_writer::LOSSLESS_IDTX_CODED.set([0; 5]);
        let (units, source) = lossless_stream(case, 1, 4, overrides);
        let coded = crate::tile_writer::LOSSLESS_IDTX_CODED.get();
        assert!(coded[3] > 0 && coded[slot] > 0, "{overrides:?}: {coded:?}");
        assert_same_samples(&decode(&units.concat()), &source, &format!("{overrides:?}"));
    }
}

const TEXT: fn(&Config, u32) -> SourceFrame = super::intra_tools::screen_frame;

const FSC: [&[(&str, &str)]; 3] = [
    &[("lossless_fsc", "1")],
    &[("lossless_fsc", "2"), ("lossless_dpcm", "1")],
    &[("lossless_fsc", "2"), ("lossless_inter_tx", "2")],
];

/// With `lossless_fsc`, intra leaves of drawn text code `fsc_mode` at
/// `lossless_tx_size` 0 and 1, on the plain IDTX coefficients (1) or on the
/// FSC coefficient path (2, next to DPCM, and next to `lossless_inter_tx`,
/// whose inter IDTX luma takes that path too); the streams decode to the
/// source samples with the same bytes on 1, 2 and 8 threads.
#[test]
fn lossless_fsc_streams_decode_to_the_source_on_any_thread_count() {
    each_case(CASES, |case| {
        for overrides in FSC {
            let name = format!("{case:?} {overrides:?}");
            crate::tile_writer::LOSSLESS_FSC_CODED.set([0; 3]);
            let (units, source) = lossless_stream_of(TEXT, case, 1, 8, overrides);
            let [small, large, inter] = crate::tile_writer::LOSSLESS_FSC_CODED.get();
            if overrides[1..]
                .iter()
                .all(|&(field, _)| field != "lossless_inter_tx")
            {
                assert!(small + large > 0, "{name}: no FSC leaf");
            } else if case.3 == 0 {
                assert!(inter > 0, "{name}: no inter FSC coefficients");
            }
            assert_same_samples(&decode(&units.concat()), &source, &name);
            for threads in [2, 8] {
                let other = lossless_stream_of(TEXT, case, threads, 8, overrides).0;
                assert_eq!(other, units, "{name} {threads}");
            }
        }
    });
}

/// Lossless FSC leaves forced to `lossless_tx_size = 1` (one IDTX per
/// `find_tx_size(Min(32, W), Min(32, H))`, § 5.20.6.1) on both coefficient
/// paths decode to the source samples.
#[test]
fn lossless_fsc_large_transforms_decode_to_the_source() {
    for case in [CASES[0], CASES[1], CASES[3]] {
        for overrides in [FSC[0], FSC[2]] {
            let name = format!("{case:?} {overrides:?}");
            crate::tile_writer::LOSSLESS_FSC_CODED.set([0; 3]);
            crate::mode_decision::FORCE_LOSSLESS_FSC.set(true);
            let (units, source) = lossless_stream_of(TEXT, case, 1, 4, overrides);
            crate::mode_decision::FORCE_LOSSLESS_FSC.set(false);
            let [_, large, _] = crate::tile_writer::LOSSLESS_FSC_CODED.get();
            assert!(large > 0, "{name}: no lossless_tx_size = 1 FSC leaf");
            assert_same_samples(&decode(&units.concat()), &source, &name);
        }
    }
}

const PALETTE: [&[(&str, &str)]; 2] = [
    &[("lossless_palette", "1")],
    &[
        ("lossless_palette", "1"),
        ("lossless_fsc", "2"),
        ("lossless_dpcm", "1"),
    ],
];

/// With `lossless_palette`, intra leaves of drawn text code palettes over
/// 4x4 Walsh-Hadamard residual units, alone and next to FSC on its
/// coefficient path and DPCM; the streams decode to the source samples with
/// the same bytes on 1, 2 and 8 threads.
#[test]
fn lossless_palette_streams_decode_to_the_source_on_any_thread_count() {
    each_case(CASES, |case| {
        for overrides in PALETTE {
            let name = format!("{case:?} {overrides:?}");
            crate::tile_writer::LOSSLESS_PALETTE_CODED.set(0);
            let (units, source) = lossless_stream_of(TEXT, case, 1, 8, overrides);
            let coded = crate::tile_writer::LOSSLESS_PALETTE_CODED.get();
            assert!(coded > 0, "{name}: no palette leaf");
            assert_same_samples(&decode(&units.concat()), &source, &name);
            for threads in [2, 8] {
                let other = lossless_stream_of(TEXT, case, threads, 8, overrides).0;
                assert_eq!(other, units, "{name} {threads}");
            }
        }
    });
}

/// AVM decodes the lossless streams, with and without DPCM,
/// `lossless_inter_tx` and `lossless_fsc`, and the palette streams of drawn
/// text, to the source samples.
#[test]
#[ignore = "needs SPLOT_AVM_DECODER"]
fn avm_decodes_lossless_streams_to_the_source() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for case in CASES {
        for overrides in [
            &[][..],
            DPCM,
            INTER_TX[0],
            INTER_TX[1],
            FSC[0],
            FSC[1],
            FSC[2],
        ] {
            let (units, source) = lossless_stream(case, 1, 8, overrides);
            let avm = avm_decode(&avmdec, &units, (96, 72), case.1, "lossless");
            assert_same_samples(&avm, &source, &format!("{case:?} {overrides:?}"));
        }
        for overrides in PALETTE {
            let (units, source) = lossless_stream_of(TEXT, case, 1, 8, overrides);
            let avm = avm_decode(&avmdec, &units, (96, 72), case.1, "lossless");
            assert_same_samples(&avm, &source, &format!("{case:?} {overrides:?}"));
        }
    }
}
