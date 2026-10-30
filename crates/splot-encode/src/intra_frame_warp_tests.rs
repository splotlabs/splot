// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Warped motion (§ 5.20.7.11 `is_warp`, § 5.20.7.14, § 5.20.7.7) in coded
//! streams. Random warp leaves of every model are covered by
//! `random_inter_leaves_decode_to_the_writer_recon`.

use super::*;
use splot_decode::encoder_syntax::{WARP_CANDIDATES, WarpModel, WarpModes};

/// A random warp prediction `leaf` can code, with a block vector near `mv`;
/// `None` where it codes none.
pub(super) fn random_warp(
    syntax: &mut IntraTileSyntax,
    leaf: &LeafBlock,
    rng: &mut Rng,
    mv: MotionVector,
) -> Option<InterPrediction> {
    let ref_frame = rng.below(syntax.reference_count() as u64) as usize;
    let options = syntax.warp_options(leaf, ref_frame).unwrap()?;
    let warp_idx = rng.below(WARP_CANDIDATES as u64) as usize;
    let mut models = vec![WarpModel::Candidate];
    if options.newmv {
        let high_precision = rng.below(2) == 0;
        let limit = if high_precision { 14 } else { 7 };
        let coded = match warp_idx {
            0 => 2,
            1 if options.six_param => 4,
            _ => 0,
        };
        let mut values = [0; 4];
        for value in values.iter_mut().take(coded) {
            *value = rng.below(2 * limit + 1) as i32 - limit as i32;
        }
        models.push(WarpModel::Delta {
            high_precision,
            values,
        });
    }
    if options.local {
        models.push(WarpModel::Local);
    }
    if options.extend {
        models.push(WarpModel::Extend);
    }
    let model = models[rng.below(models.len() as u64) as usize];
    let candidate = model == WarpModel::Candidate;
    let precisions = syntax.newmv_precisions(leaf).unwrap();
    let modes = WarpModes {
        ref_frame,
        model,
        warp_idx: match model {
            WarpModel::Candidate | WarpModel::Delta { .. } => warp_idx,
            _ => 0,
        },
        drl: if candidate {
            0
        } else {
            rng.below(syntax.drl_count() as u64) as usize
        },
        mv: (!candidate || (warp_idx < 2 && rng.below(2) == 0)).then_some(mv),
        precision: if candidate {
            None
        } else {
            precisions
                .get(rng.below(precisions.len() as u64 + 1) as usize)
                .copied()
        },
    };
    Some(InterPrediction::Warp(modes))
}

/// Frame `index` of a smooth texture zooming in by `1/64` and turning by
/// about `1/128` radian per frame around the frame centre.
fn zooming_frame(config: &Config, index: u32) -> SourceFrame {
    let max = i64::from(config.bit_depth.max_sample());
    let sizes = config.plane_sizes().unwrap();
    let (scale, turn) = (1024 - 16 * i64::from(index), 8 * i64::from(index));
    let planes = core::array::from_fn(|plane| {
        let size = sizes[plane];
        let (sx, sy) = match plane {
            0 => (0, 0),
            _ => (config.chroma.subsampling_x(), config.chroma.subsampling_y()),
        };
        let (cx, cy) = (size.width() as i64 / 2, size.height() as i64 / 2);
        (0..size.height() as i64)
            .flat_map(|y| (0..size.width() as i64).map(move |x| (x, y)))
            .map(|(x, y)| {
                let (dx, dy) = ((x - cx) << sx, (y - cy) << sy);
                let u = (dx * scale - dy * turn) / 1024 + 4096;
                let v = (dy * scale + dx * turn) / 1024 + 4096;
                let wave = |t: i64, period: i64| (t.rem_euclid(period) - period / 2).abs();
                let value =
                    (wave(u, 40) * 3 + wave(v, 28) * 4 + wave(u + v, 52) * 2 + plane as i64) * max
                        / 220;
                value.clamp(0, max) as u16
            })
            .collect()
    });
    SourceFrame::new(config, u64::from(index), planes).unwrap()
}

/// Preset 6 with warped motion.
fn with_warp() -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        warp: 1,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// A zooming mini-GOP stream with warped motion at qp 180, where the
/// encoder deblocks.
fn warp_stream(config: &Config) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, IntraFrameCoder) {
    let coder = IntraFrameCoder::with_preset(config, with_warp()).unwrap();
    super::carry::planned_stream_of(config, coder, zooming_frame, 180)
}

/// Mini-GOP streams with warped motion code `allow_warpmv_mode` frames with
/// every loop filter enabled, later frames predicting from the warp frames'
/// motion fields, and decode to the encoder recon in 4:2:0 10-bit, 4:2:2
/// 8-bit and 4:4:4 10-bit.
#[test]
fn warp_streams_decode_to_the_encoder_recon() {
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Ten),
        (PixelFormat::Yuv422, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        let mut config = config(72, 56, bit_depth, 6, 180);
        config.chroma = chroma;
        let (units, expected, cores, coder) = warp_stream(&config);
        let filter = coder.sequence.filter.unwrap();
        assert!(filter.enable_cdef && filter.enable_restoration && filter.enable_ccso);
        assert!(filter.enable_gdf, "{chroma:?}");
        let warp_frames = cores
            .iter()
            .filter(|core| {
                core.inter_tail
                    .as_ref()
                    .is_some_and(|tail| tail.allow_warpmv_mode)
            })
            .count();
        assert!(warp_frames > 1, "{chroma:?}");
        assert!(
            cores.iter().any(|core| core
                .deblocking_filter_params
                .as_ref()
                .is_some_and(|params| params.apply_deblocking_filter.iter().any(|&on| on))),
            "{chroma:?}: no deblocked frame"
        );
        assert_same_samples(&decode(&units), &expected, &format!("{chroma:?}"));
    }
}

/// A tiled warp stream codes the same bytes on 1, 2 and 8 threads and
/// decodes to the encoder recon.
#[test]
fn warp_streams_match_on_any_thread_count() {
    let stream = |threads: usize| {
        let mut config = config(136, 72, BitDepth::Eight, 6, 180);
        config.tiles = TileLayout {
            cols_log2: 1,
            rows_log2: 0,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        warp_stream(&config)
    };
    let (one, expected, ..) = stream(1);
    assert_same_samples(&decode(&one), &expected, "tiled");
    for threads in [2, 8] {
        assert!(stream(threads).0 == one, "{threads} threads");
    }
}

/// The search of a zooming inter frame picks warp leaves, and rolls back.
#[test]
fn zoom_search_picks_warp_leaves() {
    let config = config(96, 64, BitDepth::Eight, 6, 90);
    let mut coder = IntraFrameCoder::with_preset(&config, with_warp()).unwrap();
    coder
        .encode_frame(
            &zooming_frame(&config, 0),
            &crate::gop::plan(0, config.keyint),
            90,
        )
        .unwrap();
    let plan = crate::gop::plan(1, config.keyint);
    let trees = search_frame(&coder, &plan, &zooming_frame(&config, 1), config.bit_depth);
    let warp = |choice: &LeafChoice| matches!(choice.inter, Some(InterPrediction::Warp(_)));
    assert!(trees.iter().map(|tree| leaves(tree, warp)).sum::<usize>() > 0);
}

/// AVM decodes warp streams like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_warp_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Ten),
        (PixelFormat::Yuv444, BitDepth::Eight),
    ] {
        let mut config = config(72, 56, bit_depth, 6, 180);
        config.chroma = chroma;
        let (units, expected, ..) = warp_stream(&config);
        let avm = avm_decode(
            &avmdec,
            &[units],
            (config.width, config.height),
            bit_depth,
            &format!("warp{chroma:?}"),
        );
        assert!(avm == expected, "{chroma:?}: AVM output differs");
    }
}
