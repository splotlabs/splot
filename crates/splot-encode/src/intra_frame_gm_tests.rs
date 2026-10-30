// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Global motion (§ 5.18.9.1 `global_motion_params()`, `GLOBALMV` and
//! `GLOBAL_GLOBALMV` warps) in coded streams. Random `GLOBALMV` and
//! `GLOBAL_GLOBALMV` leaves over random models are covered by
//! `random_inter_leaves_decode_to_the_writer_recon`.

use super::*;

/// `ROTZOOM` models of list references 0 and 1 from `seed`: a scale and a
/// rotation within 1/32 and a translation within 4 samples, on the coded
/// lattices.
pub(super) fn random_models(seed: u64) -> [Option<[i32; 6]>; 7] {
    let mut rng = Rng(seed.rotate_left(11) | 1);
    let mut models = [None; 7];
    for model in models.iter_mut().take(2) {
        let mut pick = |range: u64| rng.below(2 * range + 1) as i32 - range as i32;
        let (scale, turn) = ((1 << 16) + (pick(32) << 6), pick(32) << 6);
        let (dx, dy) = (pick(32) << 13, pick(32) << 13);
        *model = Some([dx, dy, scale, turn, -turn, scale]);
    }
    models
}

/// Preset 6 with global motion in low-delay order, so list reference 0 of
/// every inter frame is the frame before it.
fn with_global_motion() -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        global_motion: 1,
        mini_gop_size: 1,
        lookahead_frames: 1,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// A zoom by 1/64 per frame about the frame centre of `config`, as the
/// `ROTZOOM` model of the previous frame.
fn zoom_model(config: &Config) -> [i32; 6] {
    let scale = (1 << 16) - (16 << 6);
    let shift = |size: u32| -((i64::from(size) / 2) * i64::from(scale - (1 << 16))) as i32;
    let lattice = |v: i32| (v >> 13) << 13;
    [
        lattice(shift(config.width)),
        lattice(shift(config.height)),
        scale,
        0,
        0,
        scale,
    ]
}

/// Nine zooming frames at qp 180 whose inter frames code `model` for list
/// reference 0, each coded frame with its `GLOBALMV` leaves.
fn global_motion_stream(config: &Config, model: [i32; 6]) -> super::warp::WarpStream {
    let mut coder = IntraFrameCoder::with_preset(config, with_global_motion()).unwrap();
    coder.global_motion[0] = Some(model);
    super::warp::zooming_stream(config, coder, |coded| (coded.global_leaves, 0))
}

/// Low-delay streams whose inter frames code a zoom model code `GLOBALMV`
/// leaves under every loop filter in frames whose motion records later
/// frames read, and decode to the encoder recon in 4:2:0 10-bit and 4:4:4
/// 8-bit, on 1 and 8 threads alike.
#[test]
fn global_motion_streams_decode_to_the_encoder_recon() {
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Ten),
        (PixelFormat::Yuv444, BitDepth::Eight),
    ] {
        let mut config = config(72, 56, bit_depth, 6, 180);
        config.chroma = chroma;
        let stream = global_motion_stream(&config, zoom_model(&config));
        let coded = stream
            .frames
            .iter()
            .filter_map(|(core, _)| core.inter_tail.as_ref())
            .filter(|tail| tail.global_motion.use_global_motion)
            .count();
        assert!(coded > 1, "{chroma:?}");
        super::warp::assert_warp_records_consumed(&stream.frames, &format!("{chroma:?}"));
        assert_same_samples(
            &decode(&stream.units),
            &stream.expected,
            &format!("{chroma:?}"),
        );
        config.threads = splot_parallel::ThreadCount::from(8);
        let eight = global_motion_stream(&config, zoom_model(&config));
        assert!(eight.units == stream.units, "{chroma:?}: 8 threads");
    }
}

/// AVM decodes global motion streams like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_global_motion_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let config = config(72, 56, BitDepth::Ten, 6, 180);
    let stream = global_motion_stream(&config, zoom_model(&config));
    let (units, expected) = (stream.units, stream.expected);
    let avm = avm_decode(
        &avmdec,
        &[units],
        (config.width, config.height),
        config.bit_depth,
        "global-motion",
    );
    assert!(avm == expected, "AVM output differs");
}
