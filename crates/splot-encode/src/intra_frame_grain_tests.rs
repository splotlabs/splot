// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Film grain (§ 5.14, § 5.18.10, § 7.21.7) in key and inter frames.

use super::*;
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
            let [y, cb, cr] = grain::estimate(&source);
            model = Some(encoder_film_grain_model(chroma, [&y, &cb, &cr]));
        }
        let mut coded = coder.code_frame(&source, &plan, 120, &[]).unwrap();
        let frame = &coded.reference.as_ref().unwrap().1;
        let planes = if on {
            let seed = crate::gop::grain_seed(plan.frame_index);
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
}
