// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame overlap across the mini-GOPs of a batch.

use super::*;
use crate::encoder::{BatchOutput, BatchStep};
use crate::gop::GopPlanner;
use crate::preset::PresetParams;

/// `preset` with frame overlap `on`.
fn with_overlap(preset: u8, on: bool) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        frame_overlap: on,
        ..*Preset::new(preset).unwrap().params()
    }))
}

/// `(preset, (width, height, qp), frames)`.
type OverlapCase = (u8, (u32, u32, u16), u32);

/// 64x64 superblocks in a wavefront (3 rows), 128x128 superblocks in raster
/// order (3 rows).
const OVERLAP_CASES: [OverlapCase; 2] = [(9, (200, 136, 150), 20), (6, (300, 264, 140), 12)];

/// The frames of `case` coded on `threads` threads with `params`, in batches
/// of `per_batch` mini-GOPs: the stream units in step order and the encoder
/// recon of every output frame in output order.
fn overlap_stream(
    threads: usize,
    params: &'static PresetParams,
    case: OverlapCase,
    per_batch: usize,
) -> (Vec<u8>, Vec<u8>) {
    let (_, (width, height, qp), count) = case;
    let mut config = config(width, height, BitDepth::Ten, case.0, qp);
    config.threads = splot_parallel::ThreadCount::from(threads);
    let sources: Vec<SourceFrame> = (0..count)
        .map(|index| super::tip::zooming_frame(&config, index))
        .collect();
    let mut coder = IntraFrameCoder::with_preset(&config, params).unwrap();
    coder.start_sequence(&sources[0]).unwrap();
    let mut planner = GopPlanner::new(config.keyint, params.mini_gop_size);
    let (mut units, mut shown) = (Vec::new(), BTreeMap::new());
    let mut outputs_in_order = Vec::new();
    let mut remaining = count as usize;
    while remaining > 0 {
        let mut plans = Vec::new();
        while plans.len() < per_batch
            && let Some(len) = planner.next_len(remaining, true)
        {
            plans.push(planner.plan(len).unwrap());
            remaining -= len;
        }
        let batch: Vec<Vec<Vec<BatchStep<'_>>>> = plans
            .iter()
            .map(|steps| {
                crate::gop::waves(steps)
                    .map(|wave| {
                        wave.iter()
                            .map(|step| match *step {
                                GopStep::Code(plan) => {
                                    BatchStep::Code(&sources[plan.display_index as usize], plan, qp)
                                }
                                GopStep::ShowExisting { slot, .. } => BatchStep::ShowExisting(slot),
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect();
        let outputs = coder.encode_batch(&batch).unwrap();
        for (step, output) in plans.iter().flatten().zip(outputs) {
            match (step, output) {
                (GopStep::Code(plan), BatchOutput::Coded(coded)) => {
                    assert!(coded.reference.is_none(), "committed in the batch");
                    shown.insert(plan.display_index, recon_bytes(&coded, config.bit_depth));
                    units.extend(coded.temporal_unit);
                }
                (GopStep::ShowExisting { .. }, BatchOutput::Unit(unit)) => units.extend(unit),
                _ => panic!("an output of another kind than its step"),
            }
            if let Some(display) = step.output() {
                outputs_in_order.push(display);
            }
        }
    }
    let recon = outputs_in_order
        .iter()
        .flat_map(|display| shown[display].clone())
        .collect();
    (units, recon)
}

/// Overlapped batches, with every loop filter of the preset row, decode to the
/// encoder recon, write the same bytes on 1, 2 and 8 threads and in batches of
/// any size, and differ from coding one mini-GOP at a time.
#[test]
fn overlapped_batches_decode_to_the_encoder_recon_on_any_thread_count() {
    for case in OVERLAP_CASES {
        let on = with_overlap(case.0, true);
        let (one, recon) = overlap_stream(1, on, case, 3);
        let name = format!("{case:?}");
        assert_same_samples(&decode(&one), &recon, &name);
        for threads in [2, 8] {
            assert!(
                overlap_stream(threads, on, case, 3).0 == one,
                "{name}: {threads} threads"
            );
        }
        assert!(
            overlap_stream(4, on, case, 1).0 == one,
            "{name}: one mini-GOP per batch"
        );
        let off = overlap_stream(2, with_overlap(case.0, false), case, 3).0;
        assert_ne!(off, one, "{name}: the overlap changes no byte");
    }
}

/// A preset the overlap does not model (warped motion) codes one mini-GOP at
/// a time: its stream is the stream without the overlap.
#[test]
fn ineligible_presets_keep_the_stream_without_overlap() {
    let stream = |on: bool| {
        let mut config = config(96, 64, BitDepth::Eight, 9, 160);
        config.preset_overrides = vec![
            ("frame_overlap".into(), u8::from(on).to_string()),
            ("warp".into(), "1".into()),
        ];
        let mut encoder = Encoder::new(config.clone()).unwrap();
        for index in 0..12 {
            encoder.send_frame(panning_frame(&config, index)).unwrap();
        }
        encoder.flush().unwrap();
        let mut bytes = Vec::new();
        while let Some(packet) = encoder.receive_packet().unwrap() {
            bytes.extend(packet.data);
        }
        bytes
    };
    assert!(stream(true) == stream(false));
}

/// AVM decodes overlapped batches like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_overlapped_batches_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (index, case) in OVERLAP_CASES.into_iter().enumerate() {
        let (units, recon) = overlap_stream(4, with_overlap(case.0, true), case, 3);
        let avm = avm_decode(
            &avmdec,
            &[units],
            (case.1.0, case.1.1),
            BitDepth::Ten,
            &format!("overlap{index}"),
        );
        assert!(avm == recon, "{case:?}: AVM output differs");
    }
}
