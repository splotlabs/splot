// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Superblock qindex offsets coded as `delta_q` (§ 5.20.5.11) in key and
//! inter frames.

use super::*;

/// Asymmetric qindex offsets, with the matching lambda log2, for each
/// superblock of `config`: large jumps both ways, zeros, and offsets that
/// the `delta_q_res` step does not divide.
fn offsets(config: &Config, seed: usize) -> Vec<(i16, i16)> {
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

/// `config` at constant quality with the block-level temporal model at
/// `strength` percent on `threads` threads, through the [`Encoder`].
fn tpl_stream(mut config: Config, strength: u8, threads: usize) -> (Vec<Vec<u8>>, Vec<u8>) {
    config.rate_control = RateControlMode::Crf { crf: 30 };
    config.threads = splot_parallel::ThreadCount::from(threads);
    config.preset_overrides = vec![("tpl_delta_q".into(), strength.to_string())];
    encode_stream(&config, 9)
}

/// The temporal model's superblock qindex and lambda change the stream, which
/// decodes to the encoder recon in mini-GOPs at both superblock sizes and
/// every chroma format, with the same bytes on 1, 2 and 8 threads.
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
        let mut config = config(136, 72, bit_depth, preset, 0);
        config.chroma = CHROMAS[index];
        let name = format!("preset {preset} {bit_depth:?} {:?}", config.chroma);
        let (units, recon) = tpl_stream(config.clone(), 100, 1);
        assert_same_samples(&decode(&units.concat()), &recon, &name);
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
        let (units, recon) = tpl_stream(config, 100, 2);
        let avm = avm_decode(&avmdec, &units, (200, 136), bit_depth, "tpl-delta-q");
        assert!(
            avm == recon,
            "{bit_depth:?}: AVM output of the temporal model differs"
        );
    }
}
