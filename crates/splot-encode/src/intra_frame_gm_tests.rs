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

/// Preset 6 with global motion.
fn with_global_motion() -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        global_motion: 1,
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

/// Codes nine zooming frames in mini-GOPs at qp 180 whose inter frames code
/// `model` for list reference 0; returns the units, the recon in display
/// order and each coded frame's header in coding order.
fn global_motion_stream(
    config: &Config,
    model: [i32; 6],
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>) {
    let mut coder = IntraFrameCoder::with_preset(config, with_global_motion()).unwrap();
    coder.global_motion[0] = Some(model);
    let (units, expected, cores, _) =
        super::carry::planned_stream_of(config, coder, super::warp::zooming_frame, 180);
    (units, expected, cores)
}

/// Mini-GOP streams whose inter frames code a zoom model code `GLOBALMV`
/// leaves under every loop filter, later frames take temporal vectors from
/// them, and the streams decode to the encoder recon in 4:2:0 10-bit and
/// 4:4:4 8-bit, on 1 and 8 threads alike.
#[test]
fn global_motion_streams_decode_to_the_encoder_recon() {
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Ten),
        (PixelFormat::Yuv444, BitDepth::Eight),
    ] {
        let mut config = config(72, 56, bit_depth, 6, 180);
        config.chroma = chroma;
        let (units, expected, cores) = global_motion_stream(&config, zoom_model(&config));
        let coded = cores
            .iter()
            .filter_map(|core| core.inter_tail.as_ref())
            .filter(|tail| tail.global_motion.use_global_motion)
            .count();
        assert!(coded > 1, "{chroma:?}");
        assert!(
            cores
                .iter()
                .filter_map(|core| core.inter.as_ref())
                .any(|inter| inter.use_ref_frame_mvs == Some(true)),
            "{chroma:?}"
        );
        assert_same_samples(&decode(&units), &expected, &format!("{chroma:?}"));
        config.threads = splot_parallel::ThreadCount::from(8);
        let (eight, ..) = global_motion_stream(&config, zoom_model(&config));
        assert!(eight == units, "{chroma:?}: 8 threads");
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
    let (units, expected, _) = global_motion_stream(&config, zoom_model(&config));
    let avm = avm_decode(
        &avmdec,
        &[units],
        (config.width, config.height),
        config.bit_depth,
        "global-motion",
    );
    assert!(avm == expected, "AVM output differs");
}
