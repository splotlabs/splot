// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Global motion (§ 5.18.9.1 `global_motion_params()`, `GLOBALMV` and
//! `GLOBAL_GLOBALMV` warps) in coded streams. Random `GLOBALMV` and
//! `GLOBAL_GLOBALMV` leaves over random models are covered by
//! `random_inter_leaves_decode_to_the_writer_recon`.

use super::*;
use splot_core::headers::frame::REFS_PER_FRAME;

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
        config.tiles = TileLayout {
            cols_log2: 1,
            rows_log2: 0,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        global_motion_stream(&config, false)
    };
    let one = stream(1);
    assert_global_motion_decodes(&one, "tiled");
    for threads in [2, 8] {
        let other = stream(threads);
        super::warp::assert_warp_records_consumed(&other.frames, &format!("{threads} threads"));
        assert!(other.units == one.units, "{threads} threads");
    }
}

/// The encoder's coder with the recon and the global motion use of each frame.
struct Counting {
    coder: IntraFrameCoder,
    bit_depth: BitDepth,
    recons: Arc<Mutex<BTreeMap<u64, Vec<u8>>>>,
    global_frames: Arc<Mutex<usize>>,
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
            let global = frame
                .reference
                .as_ref()
                .and_then(|(core, ..)| core.inter_tail.as_ref())
                .is_some_and(|tail| tail.global_motion.use_global_motion);
            *self.global_frames.lock().unwrap() += usize::from(global);
        }
        Ok(coded)
    }

    fn commit(&mut self, frame: &mut CodedFrame) -> Result<()> {
        self.coder.commit(frame)
    }

    fn show_existing_frame(&mut self, slot: u32) -> Result<Vec<u8>> {
        self.coder.show_existing_frame(slot)
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

/// The encoder with `global_motion = 1` estimates zoom models from its
/// lookahead for zooming content, codes them in inter frames, and the stream
/// decodes to the encoder recon.
#[test]
fn the_encoder_estimates_and_codes_global_motion() {
    let mut config = config(160, 96, BitDepth::Eight, 6, 120);
    config.preset_overrides = vec![("global_motion".to_owned(), "1".to_owned())];
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
    assert!(
        *global_frames.lock().unwrap() > 0,
        "no frame codes global motion"
    );
    let expected: Vec<u8> = recons.lock().unwrap().values().flatten().copied().collect();
    assert_same_samples(&decode(&units), &expected, "estimated");
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
