// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Superblock qindex offsets coded as `delta_q` (§ 5.20.5.11) in key and
//! inter frames.

use super::*;

/// Asymmetric qindex offsets, with the matching lambda log2, for each
/// superblock of `config`: large jumps both ways, zeros, and offsets that
/// the `delta_q_res` step does not divide.
pub(super) fn offsets(config: &Config, seed: usize) -> Vec<(i16, i16)> {
    let sb = if config.preset.params().max_block_log2 >= 7 {
        128
    } else {
        64
    };
    let count = config.width.div_ceil(sb) * config.height.div_ceil(sb);
    (0..count as usize)
        .map(|index| [-29, 0, 17, -6, 31, 3, -32, 0, 11][(index * 7 + seed) % 9])
        .map(|offset| (offset, offset * 256 / 12))
        .collect()
}

/// Two tiles of a texture that pans every second frame, so inter frames code
/// superblock-sized skipped leaves: a key frame and four inter frames on
/// `threads` threads with superblock offsets, every loop filter coded in
/// some frame. Returns the units, the encoder recon and whether each frame
/// codes `delta_q_present`.
fn delta_q_stream(mut config: Config, threads: usize) -> (Vec<Vec<u8>>, Vec<u8>, Vec<bool>) {
    config.tiles = TileLayout {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let RateControlMode::ConstantQp { qp } = config.rate_control else {
        unreachable!("these tests code at a constant qp")
    };
    let mut coder = IntraFrameCoder::new(&config).unwrap();
    let (mut units, mut recon, mut present) = (Vec::new(), Vec::new(), Vec::new());
    let mut filters = [false; 5];
    for index in 0..5 {
        let plan = crate::gop::plan(u64::from(index), config.keyint);
        let source = panning_frame(&config, index / 2);
        let mut coded = coder
            .code_frame(&source, &plan, qp, &offsets(&config, index as usize))
            .unwrap();
        let core = &coded.reference.as_ref().unwrap().0;
        present.push(core.delta_q_params.is_some_and(|dq| dq.delta_q_present));
        let used = [
            core.deblocking_filter_params
                .is_some_and(|df| df.apply_deblocking_filter.contains(&true)),
            core.cdef_params.is_some_and(|cdef| cdef.cdef_frame_enable),
            core.lr_params.as_ref().is_some_and(|lr| lr.uses_lr),
            core.ccso_params
                .as_ref()
                .is_some_and(|ccso| ccso.ccso_frame_flag == Some(true)),
            core.gdf_params.is_some_and(|gdf| gdf.gdf_frame_enable),
        ];
        filters = core::array::from_fn(|i| filters[i] || used[i]);
        coder.commit(&mut coded).unwrap();
        recon.extend(recon_bytes(&coded, config.bit_depth));
        units.push(coded.temporal_unit);
    }
    assert_eq!(filters, [true; 5], "deblock, CDEF, LR, CCSO, GDF");
    (units, recon, present)
}

/// Every chroma format and both bit depths and superblock sizes decode to the
/// encoder recon, with the same bytes on 1, 2 and 8 threads.
#[test]
fn superblock_offsets_decode_to_the_encoder_recon_on_any_thread_count() {
    for (index, (bit_depth, preset, qp)) in [
        (BitDepth::Eight, 9, 200),
        (BitDepth::Ten, 6, 240),
        (BitDepth::Eight, 10, 200),
    ]
    .into_iter()
    .enumerate()
    {
        let mut config = config(200, 136, bit_depth, preset, qp);
        config.chroma = CHROMAS[index];
        let name = format!("{:?} {bit_depth:?} preset {preset}", config.chroma);
        let (one, recon, present) = delta_q_stream(config.clone(), 1);
        assert!(present.iter().all(|&on| on), "{name}");
        assert_same_samples(&decode(&one.concat()), &recon, &name);
        for threads in [2, 8] {
            assert!(
                delta_q_stream(config.clone(), threads).0 == one,
                "{name}: {threads} threads"
            );
        }
    }
}

/// Offsets change the coded frame, no offsets code no `delta_q_present`, and
/// offsets that miss a superblock are refused.
#[test]
fn empty_offsets_code_no_delta_q() {
    let config = config(96, 64, BitDepth::Eight, 9, 150);
    let coder = IntraFrameCoder::new(&config).unwrap();
    let plan = crate::gop::plan(0, config.keyint);
    let source = panning_frame(&config, 0);
    let off = coder.code_frame(&source, &plan, 150, &[]).unwrap();
    let on = coder
        .code_frame(&source, &plan, 150, &offsets(&config, 1))
        .unwrap();
    let mut short = offsets(&config, 1);
    short.pop();
    assert!(matches!(
        coder.code_frame(&source, &plan, 150, &short),
        Err(Error::Unsupported { .. })
    ));
    let present = |coded: &CodedFrame| {
        let core = &coded.reference.as_ref().unwrap().0;
        core.delta_q_params.is_some_and(|dq| dq.delta_q_present)
    };
    assert!(!present(&off) && present(&on));
    assert_ne!(off.temporal_unit, on.temporal_unit);
}

/// Each frame's decoder output and `[delta_q_present, deblock, CDEF, LR,
/// CCSO, GDF]` by display index.
type FrameTools = Arc<Mutex<BTreeMap<u64, (Vec<u8>, [bool; 6])>>>;

/// Forwards to the frame coder and records [`FrameTools`].
struct ToolRecording {
    coder: IntraFrameCoder,
    bit_depth: BitDepth,
    frames: FrameTools,
}

impl FrameCoder for ToolRecording {
    fn encode_wave(&self, wave: &[(&SourceFrame, GopFrame, u16)]) -> Result<Vec<CodedFrame>> {
        let coded = self.coder.encode_wave(wave)?;
        for ((_, plan, _), frame) in wave.iter().zip(&coded) {
            let core = &frame.reference.as_ref().unwrap().0;
            let tools = [
                core.delta_q_params.is_some_and(|dq| dq.delta_q_present),
                core.deblocking_filter_params
                    .is_some_and(|df| df.apply_deblocking_filter.contains(&true)),
                core.cdef_params.is_some_and(|cdef| cdef.cdef_frame_enable),
                core.lr_params.as_ref().is_some_and(|lr| lr.uses_lr),
                core.ccso_params
                    .as_ref()
                    .is_some_and(|ccso| ccso.ccso_frame_flag == Some(true)),
                core.gdf_params.is_some_and(|gdf| gdf.gdf_frame_enable),
            ];
            let recon = recon_bytes(frame, self.bit_depth);
            self.frames
                .lock()
                .unwrap()
                .insert(plan.display_index, (recon, tools));
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
}

/// Nine frames of a texture that pans every second frame, at constant
/// quality with the block-level temporal model at `strength` percent on
/// `threads` threads, through the [`Encoder`]: the units, the encoder recon
/// in display order, and the coded tools of each frame.
fn tpl_stream(
    mut config: Config,
    strength: u8,
    threads: usize,
) -> (Vec<Vec<u8>>, Vec<u8>, Vec<[bool; 6]>) {
    config.rate_control = RateControlMode::Crf { crf: 50 };
    config.threads = splot_parallel::ThreadCount::from(threads);
    config.preset_overrides = vec![("tpl_delta_q".into(), strength.to_string())];
    let frames = Arc::default();
    let coder = ToolRecording {
        coder: IntraFrameCoder::new(&config).unwrap(),
        bit_depth: config.bit_depth,
        frames: Arc::clone(&frames),
    };
    let mut encoder = Encoder::with_coder(config.clone(), Box::new(coder)).unwrap();
    let mut units = Vec::new();
    for index in 0..9 {
        encoder
            .send_frame(panning_frame(&config, index / 2))
            .unwrap();
        while let Some(packet) = encoder.receive_packet().unwrap() {
            units.push(packet.data);
        }
    }
    encoder.flush().unwrap();
    while let Some(packet) = encoder.receive_packet().unwrap() {
        units.push(packet.data);
    }
    let frames = frames.lock().unwrap();
    let recon = frames
        .values()
        .flat_map(|(recon, _)| recon.iter().copied())
        .collect();
    (
        units,
        recon,
        frames.values().map(|&(_, tools)| tools).collect(),
    )
}

/// The temporal model's superblock qindex and lambda change the stream, which
/// decodes to the encoder recon in mini-GOPs at both superblock sizes and
/// every chroma format, with the same bytes on 1, 2 and 8 threads. Some
/// frames code `delta_q_present` and some do not, and every loop filter
/// codes in some frame.
#[test]
fn tpl_delta_q_streams_decode_to_the_encoder_recon_on_any_thread_count() {
    for (index, (preset, bit_depth)) in [
        (6, BitDepth::Eight),
        (9, BitDepth::Ten),
        (8, BitDepth::Eight),
    ]
    .into_iter()
    .enumerate()
    {
        let mut config = config(200, 136, bit_depth, preset, 0);
        config.chroma = CHROMAS[index];
        let name = format!("preset {preset} {bit_depth:?} {:?}", config.chroma);
        let (units, recon, tools) = tpl_stream(config.clone(), 100, 1);
        assert_same_samples(&decode(&units.concat()), &recon, &name);
        let used: [bool; 6] = core::array::from_fn(|i| tools.iter().any(|t| t[i]));
        assert_eq!(used, [true; 6], "{name}: {tools:?}");
        assert!(tools.iter().any(|t| !t[0]), "{name}: {tools:?}");
        assert!(
            units != tpl_stream(config.clone(), 0, 1).0,
            "{name}: no change"
        );
        for threads in [2, 8] {
            assert!(
                tpl_stream(config.clone(), 100, threads).0 == units,
                "{name}: {threads}"
            );
        }
    }
}

/// A frame whose superblock offsets all snap to 0 at the `delta_q_res` step
/// codes no `delta_q_present`; one offset beyond half a step codes it.
#[test]
fn offsets_that_snap_to_zero_code_no_delta_q() {
    let config = config(200, 136, BitDepth::Eight, 9, 150);
    let mut coder = IntraFrameCoder::new(&config).unwrap();
    let plan = crate::gop::plan(1, config.keyint);
    let (cols, rows) = (13, 9);
    let mut costs = vec![(1000, 0); cols * rows];
    costs[0] = (1000, 1_000_000);
    let grid = (cols, rows, 16);
    let (mut empty, mut coded) = (false, false);
    for strength in 1..=100 {
        coder.set_block_importance(crate::tpl::BlockImportance {
            grid,
            strength,
            frames: [(plan.display_index, costs.clone())].into(),
        });
        let quant = coder.superblock_quant(&plan);
        let model = crate::tpl::superblock_quant(&costs, grid, (4, 3, 64), strength);
        let snaps_to_zero = model.iter().all(|&(offset, _)| offset.abs() <= 1);
        assert_eq!(quant.is_empty(), snaps_to_zero, "{strength}: {model:?}");
        (empty, coded) = (empty || snaps_to_zero, coded || !snaps_to_zero);
    }
    assert!(empty && coded);
}

/// AVM decodes frames with superblock qindex deltas to the splot output.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_delta_q_frames_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (bit_depth, preset, qp) in [(BitDepth::Eight, 9, 200), (BitDepth::Ten, 6, 240)] {
        let config = config(200, 136, bit_depth, preset, qp);
        let (units, recon, _) = delta_q_stream(config.clone(), 2);
        let avm = avm_decode(&avmdec, &units, (200, 136), bit_depth, "delta-q");
        assert!(avm == recon, "{bit_depth:?}: AVM output differs");
        let (units, recon, _) = tpl_stream(config, 100, 2);
        let avm = avm_decode(&avmdec, &units, (200, 136), bit_depth, "tpl-delta-q");
        assert!(
            avm == recon,
            "{bit_depth:?}: AVM output of the temporal model differs"
        );
    }
}
