// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Warped motion (§ 5.20.7.11 `is_warp`, § 5.20.7.14, § 5.20.7.7) in coded
//! streams. Random warp leaves of every model are covered by
//! `random_inter_leaves_decode_to_the_writer_recon`.

use super::*;
use splot_decode::encoder_syntax::{WARP_DELTA_HIGH_MAX, WARP_DELTA_LOW_MAX, WarpModel, WarpModes};

/// A random warp prediction `leaf` can code, with a block vector near `mv`,
/// of the warp kind the coverage counts least so far; `None` where it codes
/// none.
pub(super) fn random_warp(
    syntax: &mut IntraTileSyntax,
    leaf: &LeafBlock,
    rng: &mut Rng,
    mv: MotionVector,
    coverage: &mut Coverage,
) -> Option<InterPrediction> {
    let ref_frame = rng.below(syntax.reference_count() as u64) as usize;
    let options = syntax.warp_options(leaf, ref_frame).unwrap()?;
    let available = [
        options.local,
        options.extend,
        options.newmv && options.six_param,
        options.newmv,
        true,
    ];
    let kind = (0..5)
        .filter(|&kind| available[kind])
        .min_by_key(|&kind| coverage[23 + kind])?;
    let mut values = |count: usize, limit: i32| {
        let mut values = [0; 4];
        for value in values.iter_mut().take(count) {
            *value = rng.below(2 * limit as u64) as i32 - limit;
            *value += i32::from(*value >= 0);
        }
        values
    };
    let (model, warp_idx) = match kind {
        0 => (WarpModel::Local, 0),
        1 => (WarpModel::Extend, 0),
        2 => (
            WarpModel::Delta {
                high_precision: false,
                values: values(4, WARP_DELTA_LOW_MAX),
            },
            1,
        ),
        3 => (
            WarpModel::Delta {
                high_precision: true,
                values: values(2, WARP_DELTA_HIGH_MAX),
            },
            0,
        ),
        _ => (WarpModel::Candidate, rng.below(2) as usize),
    };
    let candidate = model == WarpModel::Candidate;
    let precisions = syntax.newmv_precisions(leaf).unwrap();
    let modes = WarpModes {
        ref_frame,
        model,
        warp_idx,
        drl: if candidate {
            0
        } else {
            rng.below(syntax.drl_count() as u64) as usize
        },
        mv: Some(mv),
        precision: if candidate {
            None
        } else {
            precisions
                .get(rng.below(precisions.len() as u64 + 1) as usize)
                .copied()
        },
    };
    coverage[22] += 1;
    coverage[23 + kind] += 1;
    Some(InterPrediction::Warp(modes))
}

/// Frame `index` of a smooth texture zooming in by `1/64` and turning by
/// about `1/128` radian per frame around the frame centre.
pub(super) fn zooming_frame(config: &Config, index: u32) -> SourceFrame {
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

/// Preset 6 with warped motion, and with `change` applied.
fn with_warp(change: impl FnOnce(&mut PresetParams)) -> &'static PresetParams {
    let mut params = PresetParams {
        warp: 1,
        ..*Preset::new(6).unwrap().params()
    };
    change(&mut params);
    Box::leak(Box::new(params))
}

/// A coded warp stream: its temporal units, the recon in display order, and
/// each coded frame's header and `(warp leaves, inter leaves)` in coding order.
pub(super) struct WarpStream {
    pub(super) units: Vec<u8>,
    pub(super) expected: Vec<u8>,
    pub(super) frames: Vec<(FrameHeaderCore, (usize, usize))>,
    coder: IntraFrameCoder,
}

/// Nine zooming frames in mini-GOPs at qp 180, where the encoder deblocks.
fn warp_stream(config: &Config, preset: &PresetParams) -> WarpStream {
    let coder = IntraFrameCoder::with_preset(config, preset).unwrap();
    zooming_stream(config, coder, |coded| coded.warp_leaves)
}

/// Nine zooming frames of `coder` in mini-GOPs at qp 180, each coded frame
/// with the leaf counts `count` takes from it.
pub(super) fn zooming_stream(
    config: &Config,
    mut coder: IntraFrameCoder,
    count: impl Fn(&CodedFrame) -> (usize, usize),
) -> WarpStream {
    let mut planner = crate::gop::GopPlanner::new(config.keyint, coder.preset.mini_gop_size);
    let (mut units, mut recons, mut frames) = (Vec::new(), BTreeMap::new(), Vec::new());
    let mut queued = 9;
    while let Some(len) = planner.next_len(queued, true) {
        queued -= len;
        for step in planner.plan(len).unwrap() {
            match step {
                crate::gop::GopStep::Code(plan) => {
                    let source = zooming_frame(config, plan.display_index as u32);
                    let mut coded = coder.code_frame(&source, &plan, 180, &[]).unwrap();
                    let core = coded
                        .tip_output
                        .clone()
                        .unwrap_or_else(|| coded.reference.as_ref().unwrap().0.clone());
                    frames.push((core, count(&coded)));
                    coder.commit(&mut coded).unwrap();
                    recons.insert(plan.display_index, recon_bytes(&coded, config.bit_depth));
                    units.extend(coded.temporal_unit);
                }
                crate::gop::GopStep::ShowExisting { slot, .. } => {
                    units.extend(coder.show_existing_frame(slot).unwrap());
                }
            }
        }
    }
    WarpStream {
        units,
        expected: recons.into_values().flatten().collect(),
        frames,
        coder,
    }
}

/// Asserts that a frame codes warp leaves into a reference slot and a later
/// frame takes temporal vectors over that slot, so the decoder reads the warp
/// frame's § 7.19 motion records.
pub(super) fn assert_warp_records_consumed(
    frames: &[(FrameHeaderCore, (usize, usize))],
    case: &str,
) {
    let mut warp_slots = [false; 8];
    let mut consumed = false;
    for (core, (warp, _)) in frames {
        if let Some(inter) = &core.inter
            && inter.use_ref_frame_mvs == Some(true)
        {
            consumed |= inter
                .ref_frame_idx
                .iter()
                .any(|&slot| warp_slots[slot as usize]);
        }
        let refresh = core.refresh_frame_flags.unwrap_or(0);
        for (slot, has_warp) in warp_slots.iter_mut().enumerate() {
            if refresh & (1 << slot) != 0 {
                *has_warp = *warp > 0;
            }
        }
    }
    let warps: usize = frames.iter().map(|(_, (warp, _))| warp).sum();
    assert!(warps > 0, "{case}: no warp leaf");
    assert!(
        consumed,
        "{case}: no frame reads a warp frame's motion field"
    );
}

/// Mini-GOP streams with warped motion code warp leaves with every loop
/// filter enabled, later frames take temporal vectors from the warp frames,
/// and the streams decode to the encoder recon in 4:2:0 10-bit, 4:2:2 8-bit
/// and 4:4:4 10-bit.
#[test]
fn warp_streams_decode_to_the_encoder_recon() {
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Ten),
        (PixelFormat::Yuv422, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        let mut config = config(72, 56, bit_depth, 6, 180);
        config.chroma = chroma;
        let stream = warp_stream(&config, with_warp(|_| {}));
        let filter = stream.coder.sequence.filter.unwrap();
        assert!(filter.enable_cdef && filter.enable_restoration && filter.enable_ccso);
        assert!(filter.enable_gdf, "{chroma:?}");
        assert!(
            stream.frames.iter().any(|(core, _)| core
                .deblocking_filter_params
                .as_ref()
                .is_some_and(|params| params.apply_deblocking_filter.iter().any(|&on| on))),
            "{chroma:?}: no deblocked frame"
        );
        assert_warp_records_consumed(&stream.frames, &format!("{chroma:?}"));
        assert_same_samples(
            &decode(&stream.units),
            &stream.expected,
            &format!("{chroma:?}"),
        );
    }
}

/// A tiled warp stream codes warp leaves whose motion records a later frame
/// reads, codes the same bytes on 1, 2 and 8 threads, and decodes to the
/// encoder recon.
#[test]
fn warp_streams_match_on_any_thread_count() {
    let stream = |threads: usize| {
        let mut config = config(136, 72, BitDepth::Eight, 6, 180);
        config.tiles = TileLayout {
            cols_log2: 1,
            rows_log2: 0,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        warp_stream(&config, with_warp(|_| {}))
    };
    let one = stream(1);
    assert_warp_records_consumed(&one.frames, "tiled");
    assert_same_samples(&decode(&one.units), &one.expected, "tiled");
    for threads in [2, 8] {
        let other = stream(threads);
        assert_warp_records_consumed(&other.frames, &format!("{threads} threads"));
        assert!(other.units == one.units, "{threads} threads");
    }
}

/// An inter frame of zooming content codes warp leaves and decodes to the
/// encoder recon.
#[test]
fn zoom_frames_code_warp_leaves() {
    let config = config(96, 64, BitDepth::Eight, 6, 90);
    let mut coder = IntraFrameCoder::with_preset(&config, with_warp(|_| {})).unwrap();
    let mut units = Vec::new();
    let mut expected = Vec::new();
    let mut warps = 0;
    for frame in 0..2 {
        let plan = crate::gop::plan(u64::from(frame), config.keyint);
        let mut coded = coder
            .code_frame(&zooming_frame(&config, frame), &plan, 90, &[])
            .unwrap();
        warps += coded.warp_leaves.0;
        expected.extend(recon_bytes(&coded, config.bit_depth));
        coder.commit(&mut coded).unwrap();
        units.extend(coded.temporal_unit);
    }
    assert!(warps > 0);
    assert_same_samples(&decode(&units), &expected, "zoom");
}

/// A `force_integer_mv` warp stream codes `WARPMV` leaves only (§ 5.20.7.11),
/// which predict and store their motion by translation (§ 7.13.3.14), and
/// decodes to the encoder recon with later frames reading their motion.
#[test]
fn integer_mv_warp_streams_decode_to_the_encoder_recon() {
    let config = config(72, 56, BitDepth::Ten, 6, 180);
    let stream = warp_stream(&config, with_warp(|p| p.integer_mv = true));
    assert!(
        stream
            .frames
            .iter()
            .filter(|(core, (warp, _))| *warp > 0 && core.inter_tail.is_some())
            .all(|(core, _)| core
                .inter
                .as_ref()
                .is_some_and(|inter| inter.force_integer_mv == Some(true)))
    );
    assert_warp_records_consumed(&stream.frames, "integer mv");
    assert_same_samples(&decode(&stream.units), &stream.expected, "integer mv");
}

/// AVM decodes warp streams, with and without `force_integer_mv`, like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_warp_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (chroma, bit_depth, integer_mv) in [
        (PixelFormat::Yuv420, BitDepth::Ten, false),
        (PixelFormat::Yuv444, BitDepth::Eight, false),
        (PixelFormat::Yuv420, BitDepth::Eight, true),
    ] {
        let mut config = config(72, 56, bit_depth, 6, 180);
        config.chroma = chroma;
        let stream = warp_stream(&config, with_warp(|p| p.integer_mv = integer_mv));
        let avm = avm_decode(
            &avmdec,
            &[stream.units],
            (config.width, config.height),
            bit_depth,
            &format!("warp{chroma:?}{integer_mv}"),
        );
        assert!(
            avm == stream.expected,
            "{chroma:?} integer_mv {integer_mv}: AVM output differs"
        );
    }
}
