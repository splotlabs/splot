// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Global motion (§ 5.18.9.1 `global_motion_params()`, `GLOBALMV` and
//! `GLOBAL_GLOBALMV` warps) in coded streams. Random `GLOBALMV` and
//! `GLOBAL_GLOBALMV` leaves over random models are covered by
//! `random_inter_leaves_decode_to_the_writer_recon`.

use super::*;
use splot_core::headers::frame::{GmType, REFS_PER_FRAME};

/// `ROTZOOM` models of list references 0 and 1 from `seed`: a scale and a
/// rotation within 1/32 and a translation within 4 samples, on the coded
/// lattices.
pub(super) fn random_models(seed: u64) -> [Option<[i32; 6]>; REFS_PER_FRAME] {
    let mut rng = Rng(seed.rotate_left(11) | 1);
    let mut models = [None; REFS_PER_FRAME];
    for model in models.iter_mut().take(2) {
        let mut pick = |range: u64| rng.below(2 * range + 1) as i32 - range as i32;
        let (scale, turn) = ((1 << 16) + (pick(32) << 6), pick(32) << 6);
        let (dx, dy) = (pick(32) << 13, pick(32) << 13);
        *model = Some([dx, dy, scale, turn, -turn, scale]);
    }
    models
}

/// Preset 6 with global motion in low-delay order, so list reference 0 of
/// every inter frame is the frame before it, with `force_integer_mv` frames
/// when `integer_mv`.
fn with_global_motion(integer_mv: bool) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        global_motion: 1,
        mini_gop_size: 1,
        bitrate_mini_gop_size: 1,
        lookahead_frames: 1,
        integer_mv,
        ..fast_p6()
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

/// Nine zooming frames at qp 180 whose inter frames code the zoom model for
/// list reference 0, each coded frame with its `GLOBALMV` leaves.
fn global_motion_stream(config: &Config, integer_mv: bool) -> super::warp::WarpStream {
    let mut coder = IntraFrameCoder::with_preset(config, with_global_motion(integer_mv)).unwrap();
    coder.global_motion[0] = Some(zoom_model(config));
    super::warp::zooming_stream(config, coder, |coded| (coded.global_leaves, 0))
}

/// Asserts that `stream` codes global motion and `GLOBALMV` leaves into
/// frames whose motion records later frames read, and decodes to the
/// encoder recon.
fn assert_global_motion_decodes(stream: &super::warp::WarpStream, case: &str) {
    assert!(
        stream
            .frames
            .iter()
            .filter_map(|(core, _)| core.inter_tail.as_ref())
            .filter(|tail| tail.global_motion.use_global_motion)
            .count()
            > 1,
        "{case}"
    );
    super::warp::assert_warp_records_consumed(&stream.frames, case);
    assert_same_samples(&decode(&stream.units), &stream.expected, case);
}

/// Low-delay streams whose inter frames code a zoom model code `GLOBALMV`
/// leaves under every loop filter in frames whose motion records later
/// frames read, and decode to the encoder recon in 4:2:0 10-bit and 4:4:4
/// 8-bit, and with `force_integer_mv`.
#[test]
fn global_motion_streams_decode_to_the_encoder_recon() {
    for (chroma, bit_depth, integer_mv) in [
        (PixelFormat::Yuv420, BitDepth::Ten, false),
        (PixelFormat::Yuv444, BitDepth::Eight, false),
        (PixelFormat::Yuv420, BitDepth::Eight, true),
    ] {
        let mut config = config(72, 56, bit_depth, 6, 180);
        config.chroma = chroma;
        let case = format!("{chroma:?} integer_mv {integer_mv}");
        assert_global_motion_decodes(&global_motion_stream(&config, integer_mv), &case);
    }
}

/// A tiled global motion stream codes the same bytes on 1, 2 and 8 threads.
#[test]
fn global_motion_streams_match_on_any_thread_count() {
    let stream = |threads: usize| {
        let mut config = config(136, 72, BitDepth::Eight, 6, 180);
        config.tiles = TileLayout::Uniform {
            cols_log2: 1,
            rows_log2: 0,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        global_motion_stream(&config, false)
    };
    let one = stream(1);
    assert_global_motion_decodes(&one, "tiled");
    {
        let threads = 2;
        let other = stream(threads);
        super::warp::assert_warp_records_consumed(&other.frames, &format!("{threads} threads"));
        assert!(other.units == one.units, "{threads} threads");
    }
}

/// The encoder's coder with the recon of each frame, and the frames and
/// references that code a global model and the frames that code `our_ref !=
/// NumTotalRefs`.
struct Counting {
    coder: IntraFrameCoder,
    bit_depth: BitDepth,
    recons: Arc<Mutex<BTreeMap<u64, Vec<u8>>>>,
    global_frames: Arc<Mutex<(usize, usize, usize)>>,
}

impl FrameCoder for Counting {
    fn encode_wave(&self, wave: &[(&SourceFrame, GopFrame, u16)]) -> Result<Vec<CodedFrame>> {
        let coded = self.coder.encode_wave(wave)?;
        for ((_, plan, _), frame) in wave.iter().zip(&coded) {
            let recon = recon_bytes(frame, self.bit_depth);
            self.recons
                .lock()
                .unwrap()
                .insert(plan.display_index, recon);
            let tail = frame
                .reference
                .as_ref()
                .and_then(|(core, ..)| core.inter_tail.as_ref());
            let Some(global) = tail
                .map(|tail| &tail.global_motion)
                .filter(|global| global.use_global_motion)
            else {
                continue;
            };
            let models = global.references.iter();
            let models = models.filter(|model| model.gm_type != GmType::Identity);
            let inter = frame
                .reference
                .as_ref()
                .and_then(|(core, ..)| core.inter.as_ref());
            let predicted = global.our_ref != inter.and_then(|inter| inter.num_total_refs);
            let mut counts = self.global_frames.lock().unwrap();
            *counts = (
                counts.0 + 1,
                counts.1 + models.count(),
                counts.2 + usize::from(predicted),
            );
        }
        Ok(coded)
    }

    fn commit(&mut self, frame: &mut CodedFrame) -> Result<()> {
        self.coder.commit(frame)
    }

    fn end_leading_frames(&mut self) -> Result<()> {
        self.coder.end_leading_frames()
    }

    fn show_existing_frame(
        &mut self,
        slot: u32,
        order_hint: Option<u32>,
        leading: bool,
        display_index: u64,
    ) -> Result<Vec<u8>> {
        self.coder
            .show_existing_frame(slot, order_hint, leading, display_index)
    }

    fn set_block_importance(&mut self, importance: crate::tpl::BlockImportance) {
        self.coder.set_block_importance(importance);
    }

    fn wants_global_motion(&self) -> bool {
        self.coder.wants_global_motion()
    }

    fn set_global_motion(&mut self, models: crate::global_motion::GlobalModels) {
        self.coder.set_global_motion(models);
    }
}

/// The encoded units and the recon of nine zooming frames at `global_motion`
/// `level` and `global_motion_predict` `predict` on `threads` threads, and
/// the [`Counting`] counts.
fn estimated_stream(
    (width, height, preset, qp): (u32, u32, u8, u16),
    (level, predict): (u8, bool),
    threads: usize,
) -> (Vec<u8>, Vec<u8>, (usize, usize, usize)) {
    let mut config = config(width, height, BitDepth::Eight, preset, qp);
    config.preset_overrides = vec![
        ("global_motion".to_owned(), level.to_string()),
        ("global_motion_predict".to_owned(), predict.to_string()),
        ("short_refresh".to_owned(), "false".to_owned()),
    ];
    config.threads = splot_parallel::ThreadCount::from(threads);
    let (recons, global_frames) = (Arc::default(), Arc::default());
    let coder = Counting {
        coder: IntraFrameCoder::new(&config).unwrap(),
        bit_depth: config.bit_depth,
        recons: Arc::clone(&recons),
        global_frames: Arc::clone(&global_frames),
    };
    let mut encoder = Encoder::with_coder(config.clone(), Box::new(coder)).unwrap();
    let mut units = Vec::new();
    for index in 0..=8 {
        if index == 8 {
            encoder.flush().unwrap();
        } else {
            encoder
                .send_frame(super::warp::zooming_frame(&config, index))
                .unwrap();
        }
        while let Some(packet) = encoder.receive_packet().unwrap() {
            units.extend(packet.data);
        }
    }
    let expected: Vec<u8> = recons.lock().unwrap().values().flatten().copied().collect();
    let counts = *global_frames.lock().unwrap();
    (units, expected, counts)
}

/// The encoder with `global_motion = 1` estimates zoom models from its
/// lookahead for zooming content, codes them in inter frames, and the stream
/// decodes to the encoder recon, also at qps and presets whose
/// `GLOBAL_GLOBALMV` leaves land in frames that later frames take temporal
/// vectors from. The first stream keeps the bytes main coded (FNV-1a golden).
#[test]
fn the_encoder_estimates_and_codes_global_motion() {
    let cases = [(160, 96, 6, 120), (160, 96, 6, 90), (192, 112, 9, 90)];
    each_case(cases.into_iter().enumerate(), |(index, case)| {
        let (units, expected, (frames, ..)) = estimated_stream(case, (1, false), 1);
        assert!(frames > 0, "estimated {case:?}: no global frame");
        assert_same_samples(&decode(&units), &expected, &format!("{case:?}"));
        if index == 0 {
            assert_eq!(fnv1a(&units), LEVEL_1_GOLDEN, "{case:?}");
        }
    });
}

/// The FNV-1a hash of the first `global_motion = 1` stream, recorded from main
/// 44b2ed044.
const LEVEL_1_GOLDEN: u64 = 15_796_104_231_302_014_708;

pub(super) fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// With `global_motion = 2` the encoder codes estimated models in zooming
/// frames and the streams decode to the encoder recon, with the same bytes on
/// 1, 2 and 8 threads.
#[test]
fn the_quarter_sample_estimator_codes_global_motion_on_any_thread_count() {
    each_case([(160, 96, 6, 120), (192, 112, 9, 90)], |case| {
        let (units, expected, (frames, references, _)) = estimated_stream(case, (2, false), 1);
        assert!(
            frames > 0 && references >= frames,
            "{case:?}: {frames} {references}"
        );
        assert_same_samples(&decode(&units), &expected, &format!("{case:?}"));
        {
            let threads = 2;
            assert!(
                estimated_stream(case, (2, false), threads).0 == units,
                "{case:?} {threads}"
            );
        }
    });
}

/// With `global_motion_predict` the estimated streams code frames with
/// `our_ref != NumTotalRefs` (§ 5.18.9.1) and the same recon in fewer bytes,
/// decode to that recon, and code the same bytes on 1, 2 and 8 threads.
#[test]
fn global_motion_predict_changes_only_the_parameter_bits() {
    each_case([(160, 96, 6, 120), (192, 112, 9, 90)], |case| {
        let (plain, plain_recon, plain_counts) = estimated_stream(case, (2, false), 1);
        let (units, expected, counts) = estimated_stream(case, (2, true), 1);
        assert!(counts.2 > 0 && plain_counts.2 == 0, "{case:?}: {counts:?}");
        assert_eq!(
            counts.0..=counts.1,
            plain_counts.0..=plain_counts.1,
            "{case:?}"
        );
        assert!(expected == plain_recon, "{case:?}: recon changed");
        assert!(units.len() < plain.len(), "{case:?}");
        assert_same_samples(&decode(&units), &expected, &format!("{case:?}"));
        {
            let threads = 2;
            assert!(
                estimated_stream(case, (2, true), threads).0 == units,
                "{case:?} {threads}"
            );
        }
    });
}

/// A mini-GOP zooming stream whose compound leaves code `GLOBAL_GLOBALMV`
/// wherever they can, with the zoom model on list reference 0 alone, stores
/// the model of the global list in the § 7.22 motion records as the decoder
/// does: later frames read those records, and the stream decodes to the
/// encoder recon. The estimated streams cover two global lists.
#[test]
fn global_global_leaves_store_the_model_of_each_global_list() {
    let mut config = config(128, 96, BitDepth::Eight, 6, 180);
    config.threads = splot_parallel::ThreadCount::from(1);
    let preset = Box::leak(Box::new(PresetParams {
        global_motion: 1,
        compound_skip_sad: 0,
        ..fast_p6()
    }));
    let mut coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    coder.global_motion[0] = Some(zoom_model(&config));
    crate::mode_decision::FORCE_GLOBAL_GLOBAL.set(true);
    let stream = super::warp::zooming_stream(&config, coder, |coded| (coded.global_leaves, 0));
    crate::mode_decision::FORCE_GLOBAL_GLOBAL.set(false);
    super::warp::assert_warp_records_consumed(&stream.frames, "one global list");
    assert_same_samples(&decode(&stream.units), &stream.expected, "one global list");
}

/// AVM decodes global motion streams, with and without `force_integer_mv`,
/// like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_global_motion_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for integer_mv in [false, true] {
        let config = config(72, 56, BitDepth::Ten, 6, 180);
        let stream = global_motion_stream(&config, integer_mv);
        let avm = avm_decode(
            &avmdec,
            &[stream.units],
            (config.width, config.height),
            config.bit_depth,
            &format!("global-motion-{integer_mv}"),
        );
        assert!(
            avm == stream.expected,
            "integer_mv {integer_mv}: AVM output differs"
        );
    }
}

/// AVM decodes a `global_motion = 2` stream with `global_motion_predict`
/// like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_estimated_global_motion_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let (units, expected, _) = estimated_stream((160, 96, 6, 120), (2, true), 1);
    let avm = avm_decode(&avmdec, &[units], (160, 96), BitDepth::Eight, "gm-2");
    assert!(avm == expected, "AVM output differs");
}
