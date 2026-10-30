// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Film grain (§ 5.14, § 5.18.10, § 7.21.7) in key and inter frames.

use super::*;
use crate::encoder::Encoder;
use splot_core::types::ObuType;
use splot_recon::apply_film_grain;

/// [`synthetic_frame`] with seeded noise of about 4 8-bit samples on every
/// sample, so the grain model has points.
fn grainy_frame(config: &Config, seed: u32) -> SourceFrame {
    let frame = synthetic_frame(config, seed);
    let max = i32::from(config.bit_depth.max_sample());
    let scale = 1 << (config.bit_depth.bits() - 8);
    let mut state = seed.wrapping_mul(747_796_405) | 1;
    let planes = frame.planes().clone().map(|plane| {
        plane
            .into_iter()
            .map(|sample| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let noise = (state % 15) as i32 - 7;
                (i32::from(sample) + noise * scale).clamp(0, max) as u16
            })
            .collect()
    });
    SourceFrame::new(config, frame.pts(), planes).unwrap()
}

/// Key, inter, key and inter frames on `threads` threads with grain `on`:
/// the units, the decoded output that § 7.21.7 adds to the encoder recon,
/// and the recon itself.
fn grain_stream(mut config: Config, threads: usize, on: bool) -> (Vec<Vec<u8>>, Vec<u8>, Vec<u8>) {
    config.keyint = core::num::NonZeroU32::new(2).unwrap();
    config.film_grain = on;
    config.threads = splot_parallel::ThreadCount::from(threads);
    let chroma = ChromaFormatIdc::try_new(u32::from(config.chroma.chroma_format_idc())).unwrap();
    let mut coder = IntraFrameCoder::new(&config).unwrap();
    let (mut units, mut grained, mut recon) = (Vec::new(), Vec::new(), Vec::new());
    let mut model = None;
    for index in 0..4u32 {
        let source = grainy_frame(&config, 11 + index);
        let plan = crate::gop::plan(u64::from(index), config.keyint);
        if plan.frame_type == FrameType::Key {
            let [y, cb, cr] = grain::estimate(&source, config.chroma).unwrap();
            model = Some(encoder_film_grain_model(chroma, [&y, &cb, &cr]).unwrap());
        }
        let mut coded = coder.code_frame(&source, &plan, 120, &[]).unwrap();
        let frame = &coded.reference.as_ref().unwrap().1;
        let planes = if on {
            let seed = crate::gop::coded_grain_seed(plan.frame_index);
            visible_planes(&apply_film_grain(frame, model.as_ref().unwrap(), seed).unwrap())
        } else {
            visible_planes(frame)
        };
        let planes = planes.unwrap();
        grained.extend(raw_bytes(
            planes.iter().map(Vec::as_slice),
            config.bit_depth,
        ));
        recon.extend(recon_bytes(&coded, config.bit_depth));
        units.push(coded.temporal_unit.clone());
        coder.commit(&mut coded).unwrap();
    }
    (units, grained, recon)
}

/// With grain on, `splot decode` outputs the encoder recon plus the § 7.21.7
/// grain of the key frame's model in every chroma format and both bit depths,
/// with the same bytes on 1, 2 and 8 threads; the recon equals the recon of
/// the same stream without grain, which decodes to its recon.
#[test]
fn grain_streams_decode_to_the_recon_plus_grain() {
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv422, BitDepth::Eight),
        (PixelFormat::Yuv422, BitDepth::Ten),
        (PixelFormat::Yuv444, BitDepth::Eight),
        (PixelFormat::Monochrome, BitDepth::Ten),
    ] {
        let mut config = config(72, 40, bit_depth, 6, 120);
        config.chroma = chroma;
        let name = format!("{chroma:?} {bit_depth:?}");
        let (units, grained, recon) = grain_stream(config.clone(), 1, true);
        assert_ne!(grained, recon, "{name}: no grain");
        assert_same_samples(&decode(&units.concat()), &grained, &name);
        let (plain, plain_grained, plain_recon) = grain_stream(config.clone(), 1, false);
        assert_eq!(plain_recon, recon, "{name}: grain changed the recon");
        assert_same_samples(&decode(&plain.concat()), &plain_grained, &name);
        for threads in [2, 8] {
            assert!(
                grain_stream(config.clone(), threads, true).0 == units,
                "{name}: {threads}"
            );
        }
    }
}

/// Frames through the [`Encoder`] at preset 6, whose mini-GOPs output
/// hidden alt-refs through show-existing frames, with grain `on`.
fn mini_gop_units(on: bool, noisy: bool) -> Vec<Vec<u8>> {
    let mut config = config(72, 40, BitDepth::Eight, 6, 120);
    config.film_grain = on;
    let mut encoder = Encoder::new(config.clone()).unwrap();
    for index in 0..9 {
        let frame = if noisy {
            grainy_frame(&config, 31 + index)
        } else {
            let sizes = config.plane_sizes().unwrap();
            let planes = core::array::from_fn(|plane| {
                let width = sizes.get(plane).map_or(1, |size| size.width());
                let count = sizes
                    .get(plane)
                    .map_or(0, |size| size.width() * size.height());
                (0..count)
                    .map(|i| (40 + i % width + i / width + index as usize) as u16)
                    .collect()
            });
            SourceFrame::new(&config, u64::from(index), planes).unwrap()
        };
        encoder.send_frame(frame).unwrap();
    }
    encoder.flush().unwrap();
    let mut units = Vec::new();
    while let Some(packet) = encoder.receive_packet().unwrap() {
        units.push(packet.data);
    }
    units
}

/// Every output frame of a mini-GOP stream, show-existing frames included,
/// gets grain with its own seed: no two frames carry the same noise. A clean
/// source codes no grain model, so its frames decode with `apply_grain = 0`.
#[test]
fn every_output_frame_gets_its_own_grain() {
    let frame_len = 72 * 40 * 3 / 2;
    let plain = decode(&mini_gop_units(false, true).concat());
    let grained = decode(&mini_gop_units(true, true).concat());
    assert_eq!((plain.len(), grained.len()), (9 * frame_len, 9 * frame_len));
    let noise: Vec<Vec<i16>> = plain
        .chunks(frame_len)
        .zip(grained.chunks(frame_len))
        .map(|(a, b)| {
            a.iter()
                .zip(b)
                .map(|(&a, &b)| i16::from(b) - i16::from(a))
                .collect()
        })
        .collect();
    for (index, frame) in noise.iter().enumerate() {
        assert!(frame.iter().any(|&d| d != 0), "frame {index} has no grain");
        assert!(
            !noise[..index].contains(frame),
            "frame {index} repeats a seed"
        );
    }
    for n in 0..1000 {
        assert_ne!(crate::gop::coded_grain_seed(n), 0);
        assert!(
            (0..1000).all(|m| crate::gop::coded_grain_seed(n) != crate::gop::shown_grain_seed(m))
        );
    }
    let clean = mini_gop_units(true, false);
    assert_eq!(decode(&clean.concat()).len(), 9 * frame_len);
    for unit in &clean {
        let obus = splot_core::annexb::parse_annex_b_obus_partial(unit).obus;
        assert!(
            obus.iter()
                .all(|obu| obu.header.obu_type != ObuType::FilmGrain)
        );
    }
}

/// AVM synthesizes the same grain as splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_grain_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let (units, grained, _) = grain_stream(config(72, 40, bit_depth, 6, 120), 1, true);
        let avm = avm_decode(&avmdec, &units, (72, 40), bit_depth, "grain");
        assert!(avm == grained, "{bit_depth:?}: AVM output differs");
    }
    let units = mini_gop_units(true, true);
    let avm = avm_decode(&avmdec, &units, (72, 40), BitDepth::Eight, "grain-sef");
    assert!(
        avm == decode(&units.concat()),
        "mini-GOP: AVM output differs"
    );
}
