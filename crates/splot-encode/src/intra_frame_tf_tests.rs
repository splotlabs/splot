// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Temporal filtering of key and hidden base frames through the [`Encoder`].

use super::*;

/// Frame `index` of [`panning_frame`] with seeded noise that changes every
/// frame, more in chroma than in luma.
fn noisy_frame(config: &Config, index: u32) -> SourceFrame {
    let max = config.bit_depth.max_sample();
    let clean = panning_frame(config, index);
    let mut state = (index + 1).wrapping_mul(2_654_435_761) | 1;
    let planes = core::array::from_fn(|plane| {
        let spread = u32::from(max) / 48 * (2 + plane as u32) / 2;
        clean.planes()[plane]
            .iter()
            .map(|&sample| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let noisy =
                    i64::from(sample) + i64::from(state % (2 * spread + 1)) - i64::from(spread);
                noisy.clamp(0, i64::from(max)) as u16
            })
            .collect()
    });
    SourceFrame::new(config, u64::from(index), planes).unwrap()
}

/// Each frame's decoder output, `[deblock, CDEF, LR, CCSO, GDF]` and the
/// source it was coded from, by display index.
type Coded = Arc<Mutex<BTreeMap<u64, (Vec<u8>, [bool; 5], [Vec<u16>; 3])>>>;

struct SourceRecording {
    coder: IntraFrameCoder,
    bit_depth: BitDepth,
    frames: Coded,
}

impl FrameCoder for SourceRecording {
    fn encode_wave(&self, wave: &[(&SourceFrame, GopFrame, u16)]) -> Result<Vec<CodedFrame>> {
        let coded = self.coder.encode_wave(wave)?;
        for ((source, plan, _), frame) in wave.iter().zip(&coded) {
            let core = frame
                .reference
                .as_ref()
                .map(|reference| &reference.0)
                .or(frame.tip_output.as_ref())
                .unwrap();
            let tools = [
                core.deblocking_filter_params
                    .is_some_and(|df| df.apply_deblocking_filter.contains(&true)),
                core.cdef_params.is_some_and(|cdef| cdef.cdef_frame_enable),
                core.lr_params.as_ref().is_some_and(|lr| lr.uses_lr),
                core.ccso_params
                    .as_ref()
                    .is_some_and(|ccso| ccso.ccso_frame_flag == Some(true)),
                core.gdf_params.is_some_and(|gdf| gdf.gdf_frame_enable),
            ];
            let entry = (
                recon_bytes(frame, self.bit_depth),
                tools,
                source.planes().clone(),
            );
            self.frames
                .lock()
                .unwrap()
                .insert(plan.display_index, entry);
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

    fn pool(&self) -> Option<splot_parallel::WorkerPool> {
        self.coder.pool()
    }

    fn set_open_loop_motion(&mut self, motion: crate::lookahead::OpenLoopMotion) {
        self.coder.set_open_loop_motion(motion);
    }
}

/// A filtered stream: its units, the encoder recon in display order, the
/// loop filters coded in some frame, the display indices coded from a
/// filtered source, and the source each frame was coded from.
struct Stream {
    units: Vec<Vec<u8>>,
    recon: Vec<u8>,
    tools: [bool; 5],
    filtered: Vec<u64>,
    sources: BTreeMap<u64, [Vec<u16>; 3]>,
}

/// Nine noisy panning frames with the preset fields `sets` on `threads`
/// threads, in two tiles, received after each send.
fn tf_stream(config: Config, sets: &[(&str, &str)], threads: usize) -> Stream {
    tf_stream_of(config, sets, threads, false, &noisy_frame)
}

/// Nine frames of `make` as [`tf_stream`] codes them; a `bulk` caller sends
/// every frame before it receives.
fn tf_stream_of(
    mut config: Config,
    sets: &[(&str, &str)],
    threads: usize,
    bulk: bool,
    make: &dyn Fn(&Config, u32) -> SourceFrame,
) -> Stream {
    config.threads = splot_parallel::ThreadCount::from(threads);
    config.tiles = TileLayout {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.preset_overrides = sets
        .iter()
        .map(|&(field, value)| (field.into(), value.into()))
        .collect();
    let frames = Arc::default();
    let coder = SourceRecording {
        coder: IntraFrameCoder::new(&config).unwrap(),
        bit_depth: config.bit_depth,
        frames: Arc::clone(&frames),
    };
    let mut encoder = Encoder::with_coder(config.clone(), Box::new(coder)).unwrap();
    let mut units = Vec::new();
    for index in 0..9 {
        encoder.send_frame(make(&config, index)).unwrap();
        while !bulk && let Some(packet) = encoder.receive_packet().unwrap() {
            units.push(packet.data);
        }
    }
    encoder.flush().unwrap();
    while let Some(packet) = encoder.receive_packet().unwrap() {
        units.push(packet.data);
    }
    let frames = frames.lock().unwrap();
    let filtered = frames
        .iter()
        .filter(|&(&display, (_, _, source))| source != make(&config, display as u32).planes())
        .map(|(&display, _)| display)
        .collect();
    Stream {
        units,
        recon: frames.values().flat_map(|f| f.0.iter().copied()).collect(),
        tools: core::array::from_fn(|i| frames.values().any(|f| f.1[i])),
        filtered,
        sources: frames.iter().map(|(&d, f)| (d, f.2.clone())).collect(),
    }
}

const ON: [(&str, &str); 1] = [("tf_frames", "2,2,0,0")];

/// With the filter on, the key frame and the hidden base frames code a
/// filtered source; the frames between them predict from those frames, every
/// loop filter codes in some frame, and the stream decodes to the encoder
/// recon with the same bytes on 1, 2 and 8 threads, in mini-GOPs of 4 and 2
/// and in 4:2:0 8-bit and 4:2:2 10-bit.
#[test]
fn filtered_streams_decode_to_the_encoder_recon_on_any_thread_count() {
    for (preset, bit_depth, chroma, qp, bases) in [
        (6, BitDepth::Eight, PixelFormat::Yuv420, 160, vec![0, 4, 8]),
        (
            9,
            BitDepth::Ten,
            PixelFormat::Yuv422,
            240,
            vec![0, 2, 4, 6, 8],
        ),
    ] {
        let mut config = config(200, 136, bit_depth, preset, qp);
        config.chroma = chroma;
        let name = format!("preset {preset} {bit_depth:?} {chroma:?}");
        let stream = tf_stream(config.clone(), &ON, 1);
        assert_eq!(stream.filtered, bases, "{name}");
        assert_eq!(stream.tools, [true; 5], "{name}");
        assert_same_samples(&decode(&stream.units.concat()), &stream.recon, &name);
        assert!(
            tf_stream(config.clone(), &[], 1).units != stream.units,
            "{name}"
        );
        for threads in [2, 8] {
            let other = tf_stream(config.clone(), &ON, threads);
            assert!(other.units == stream.units, "{name}: {threads} threads");
        }
    }
}

/// A zero frame count or strength codes the stream of the filter off, and
/// the preset rows filter key frames only.
#[test]
fn a_zero_count_or_strength_codes_the_unfiltered_stream() {
    let stream =
        |sets: &[(&str, &str)]| tf_stream(config(72, 40, BitDepth::Eight, 9, 255), sets, 1);
    let off = stream(&[("tf_frames", "0")]);
    assert!(off.filtered.is_empty());
    assert_eq!(stream(&[]).filtered, [0], "preset 9 row");
    assert!(stream(&ON).units != off.units, "the filter is on");
    let zero_strength = [ON[0], ("tf_strength", "0")];
    assert!(stream(&zero_strength).units == off.units, "zero strength");
    let zero_count = [("tf_frames", "0"), ("tf_strength", "255")];
    assert!(stream(&zero_count).units == off.units, "zero count");
}

/// A caller that sends every frame before it receives codes the same bytes
/// as one that receives after each send: the filter reads only the
/// lookahead window, also with as many key-frame neighbours as the window
/// holds frames.
#[test]
fn bulk_and_interleaved_callers_code_the_same_stream() {
    for (preset, keys) in [(9, "4,2,0,0"), (6, "4,2,0,0"), (9, "8,2,0,0")] {
        let config = config(96, 64, BitDepth::Eight, preset, 200);
        let sets = [("tf_frames", keys)];
        let interleaved = tf_stream_of(config.clone(), &sets, 1, false, &noisy_frame);
        let bulk = tf_stream_of(config, &sets, 1, true, &noisy_frame);
        assert!(!interleaved.filtered.is_empty(), "preset {preset}");
        assert!(
            bulk.units == interleaved.units,
            "preset {preset} keys {keys}"
        );
    }
}

/// Frame `index` of a noisy pan that cuts to another scene at frame 3: the
/// scene is `variant` from there on.
fn cut_frame(config: &Config, index: u32, variant: u32) -> SourceFrame {
    if index < 3 {
        return noisy_frame(config, index);
    }
    let scene = synthetic_frame(config, 100 * variant + index);
    SourceFrame::new(config, u64::from(index), scene.planes().clone()).unwrap()
}

/// A key frame before a scene cut reads no frame of the next scene: with
/// either scene after the cut it codes the same filtered source.
#[test]
fn key_frames_do_not_read_across_a_scene_cut() {
    let config = config(96, 64, BitDepth::Eight, 9, 200);
    let sets = [("tf_frames", "4,0,0,0")];
    let streams: Vec<Stream> = [1, 2]
        .map(|variant| {
            tf_stream_of(config.clone(), &sets, 1, false, &|c, i| {
                cut_frame(c, i, variant)
            })
        })
        .into();
    for stream in &streams {
        assert!(stream.filtered.contains(&0), "{:?}", stream.filtered);
    }
    assert!(
        streams[0].sources[&0] == streams[1].sources[&0],
        "the next scene reached the key frame"
    );
}

/// AVM decodes filtered streams to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_filtered_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (preset, bit_depth, qp) in [(6, BitDepth::Eight, 160), (9, BitDepth::Ten, 240)] {
        let config = config(200, 136, bit_depth, preset, qp);
        let stream = tf_stream(config, &ON, 2);
        assert!(!stream.filtered.is_empty());
        let avm = avm_decode(&avmdec, &stream.units, (200, 136), bit_depth, "tf");
        assert!(avm == stream.recon, "preset {preset}: AVM output differs");
    }
}
