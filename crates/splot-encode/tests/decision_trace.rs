// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The decision trace through the public encoder API.

#![expect(clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::ops::Range;

use splot_decode::block_evidence::{
    BlockEvidenceData, BlockEvidenceKind, BlockEvidenceReport, BlockEvidenceRequest,
    BlockEvidenceStatus, BlockPrediction,
};
use splot_decode::{DataProps, Decoder, DecoderSettings, SendStatus};
use splot_encode::trace::{
    BlockEvidenceId, CommittedData, FrameAttempt, FrameOutcome, LeafPrediction, ModeOutcome,
    ModeStage, Partition, PartitionOutcome, PartitionPruning, TrialCost,
};
use splot_encode::{
    BitDepth, Config, DecisionEvent, DecisionKind, DecisionTrace, DecisionTraceRequest,
    DecisionTraceStatus, DecisionTraceStop, Encoder, Error, Packet, PlaneRect, Preset,
    RateControlMode, SourceFrame, ThreadCount, TileLayout,
};

const KINDS: [DecisionKind; 3] = [
    DecisionKind::PartitionTrials,
    DecisionKind::ModeTrials,
    DecisionKind::CommittedBlocks,
];

/// A request for every event of the frames `frames`.
fn everything(frames: Range<u64>) -> DecisionTraceRequest {
    limited(frames, usize::MAX, usize::MAX)
}

/// [`everything`] within `max_events` events and `max_bytes` bytes.
fn limited(frames: Range<u64>, max_events: usize, max_bytes: usize) -> DecisionTraceRequest {
    KINDS.into_iter().fold(
        DecisionTraceRequest::new(frames, max_events, max_bytes),
        DecisionTraceRequest::with_kind,
    )
}

/// A `width x height` configuration at preset 9.
fn config(width: u32, height: u32, bit_depth: BitDepth) -> Config {
    let mut config = Config::new(width, height);
    config.bit_depth = bit_depth;
    config.preset = Preset::new(9).unwrap();
    config.rate_control = RateControlMode::ConstantQp { qp: 120 };
    config
}

/// Frame `index` of a blocky texture that pans two samples a frame.
fn source(config: &Config, index: u64) -> SourceFrame {
    let shift = config.bit_depth.bits() - 8;
    let mut plane = 0;
    let planes = config.plane_sizes().unwrap().into_iter().map(|size| {
        plane += 1;
        let (width, height) = (size.width(), size.height());
        (0..width * height)
            .map(|at| {
                let (x, y) = (at % width + 2 * index as usize, at / width);
                let block = (x / 8 * 37 + y / 8 * 91 + plane * 50) % 160;
                ((block + (x * 7 + y * 13) % 24 + 20) as u16) << shift
            })
            .collect()
    });
    let planes: Vec<Vec<u16>> = planes.collect();
    SourceFrame::new(config, index, planes.try_into().unwrap()).unwrap()
}

/// The packets of `frames` frames of [`source`], and the trace of `request`.
fn encode(
    config: &Config,
    frames: u64,
    request: Option<DecisionTraceRequest>,
) -> (Vec<Packet>, Option<DecisionTrace>) {
    let mut encoder = Encoder::new(config.clone()).unwrap();
    if let Some(request) = request {
        encoder.start_decision_trace(request);
    }
    let mut packets = Vec::new();
    for index in 0..=frames {
        if index == frames {
            encoder.flush().unwrap();
        } else {
            encoder.send_frame(source(config, index)).unwrap();
        }
        while let Some(packet) = encoder.receive_packet().unwrap() {
            packets.push(packet);
        }
    }
    (packets, encoder.take_decision_trace())
}

/// A CBR buffer small enough that the first frames are coded again.
const SMALL_BUFFER: RateControlMode = RateControlMode::Cbr {
    bitrate_kbps: 60,
    buffer_ms: 250,
    initial_ms: 125,
};

/// The frame events of `trace`, each with the block events after it.
fn frames(trace: &DecisionTrace) -> Vec<(FrameAttempt, &[DecisionEvent])> {
    let starts: Vec<usize> = (0..trace.events.len())
        .filter(|&at| matches!(trace.events[at], DecisionEvent::Frame(_)))
        .collect();
    let ends = starts.iter().skip(1).copied().chain([trace.events.len()]);
    starts
        .iter()
        .zip(ends)
        .map(|(&start, end)| match trace.events[start] {
            DecisionEvent::Frame(frame) => (frame, &trace.events[start + 1..end]),
            _ => unreachable!(),
        })
        .collect()
}

/// The decoder's partition and block records of every coded frame of
/// `packets`, sent with the packet index as user data.
fn decoder_evidence(packets: &[Packet]) -> Vec<BlockEvidenceReport> {
    let request = BlockEvidenceRequest::new(|_| true, None, 1 << 20, 1 << 28)
        .with_frame_limit(NonZeroUsize::new(64).unwrap())
        .with_kind(BlockEvidenceKind::PartitionsAndBlocks);
    let (mut decoder, capture) =
        Decoder::new_with_block_evidence(DecoderSettings::default(), request).unwrap();
    for (index, packet) in packets.iter().enumerate() {
        let props = DataProps::default().with_user_data(index as u64);
        while decoder.send_data(&packet.data, props).unwrap() == SendStatus::Full {
            while decoder.get_picture().unwrap().is_some() {}
        }
    }
    decoder.drain();
    while decoder.get_picture().unwrap().is_some() {}
    let reports: Vec<_> = core::iter::from_fn(|| capture.take())
        .filter(|report| report.frame.is_some())
        .collect();
    for report in &reports {
        assert_eq!(report.status, BlockEvidenceStatus::Complete);
    }
    reports
}

/// The corpus: a key frame and a mini-GOP at both bit depths, at a constant
/// quantizer and through the re-coding CBR buffer.
fn corpus() -> Vec<Config> {
    let mut configs = Vec::new();
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        for rate_control in [RateControlMode::ConstantQp { qp: 120 }, SMALL_BUFFER] {
            let mut config = config(96, 64, bit_depth);
            config.rate_control = rate_control;
            configs.push(config);
        }
    }
    configs
}

#[test]
fn a_capture_does_not_change_the_packets() {
    for config in corpus() {
        let (plain, none) = encode(&config, 5, None);
        assert!(none.is_none());
        let (traced, trace) = encode(&config, 5, Some(everything(0..u64::MAX)));
        assert_eq!(traced, plain, "{config:?}");
        let trace = trace.unwrap();
        assert_eq!(trace.status, DecisionTraceStatus::Complete);
        assert_eq!(trace.dropped_events, 0);
        let (truncated, short) = encode(&config, 5, Some(limited(0..u64::MAX, 300, usize::MAX)));
        assert_eq!(truncated, plain, "{config:?}");
        assert_eq!(short.unwrap().events, trace.events[..300]);
        let mut threaded = config.clone();
        threaded.threads = ThreadCount::from(3);
        let (parallel, same) = encode(&threaded, 5, Some(everything(0..u64::MAX)));
        assert_eq!(parallel, plain, "{config:?}");
        assert_eq!(same.unwrap(), trace, "{config:?}");
    }
}

#[test]
fn committed_nodes_are_the_decoder_nodes_and_trials_stay_apart() {
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let config = config(96, 64, bit_depth);
        let (packets, trace) = encode(&config, 5, Some(everything(0..u64::MAX)));
        let trace = trace.unwrap();
        let coded = frames(&trace);
        let reports = decoder_evidence(&packets);
        assert_eq!(coded.len(), 5);
        assert_eq!(reports.len(), coded.len());
        let displayed: Vec<u64> = coded.iter().map(|(f, _)| f.plan.display_index).collect();
        assert_ne!(displayed, [0, 1, 2, 3, 4], "coding order is display order");
        let (mut trials_elsewhere, mut outcomes) = (0, BTreeSet::new());
        for (index, ((frame, events), report)) in coded.iter().zip(&reports).enumerate() {
            assert_eq!(frame.plan.frame_index, index as u64);
            let FrameOutcome::Committed(output) = frame.outcome else {
                panic!("frame {index} is not committed");
            };
            let at = report.frame.unwrap().provenance.coded;
            assert_eq!(at.user_data, output.packet);
            let bytes = output.offset..output.offset + (frame.bits / 8) as usize;
            assert!(bytes.contains(&(at.offset.get() as usize)), "frame {index}");
            assert!(bytes.end <= packets[output.packet as usize].data.len());

            let mut decoded = (BTreeSet::new(), BTreeSet::new());
            for record in &report.records {
                match record.data {
                    BlockEvidenceData::Partition(partition) => {
                        decoded.0.insert(key(record.id, partition as u8));
                    }
                    BlockEvidenceData::Block(facts) => {
                        let kind = match facts.prediction {
                            BlockPrediction::Intra(_) => 0,
                            BlockPrediction::Inter(_) => 1,
                            other => panic!("frame {index}: {other:?}"),
                        };
                        decoded.1.insert(key(record.id, kind));
                    }
                    _ => {}
                }
            }
            let mut committed = (BTreeSet::new(), BTreeSet::new());
            let (mut selected, mut leaves) = (BTreeSet::new(), BTreeSet::new());
            let mut lambda_q16 = 0;
            for event in *events {
                match event {
                    DecisionEvent::Superblock(search) => lambda_q16 = search.lambda_q16,
                    DecisionEvent::Committed(node) => match node.data {
                        CommittedData::Partition(partition) => {
                            committed.0.insert(key(node.block, partition as u8));
                        }
                        CommittedData::Block(candidate) => {
                            let kind = match candidate.prediction {
                                LeafPrediction::Intra { .. } => 0,
                                LeafPrediction::Inter(_) => 1,
                                other => panic!("frame {index}: {other:?}"),
                            };
                            committed.1.insert(key(node.block, kind));
                            leaves.insert(key(node.block, Partition::None as u8));
                        }
                        _ => {}
                    },
                    DecisionEvent::Partition(trial) => {
                        outcomes.insert(format!("{:?}", trial.outcome));
                        if let Some(cost) = trial.cost {
                            assert_cost(cost, lambda_q16);
                        }
                        if trial.outcome == PartitionOutcome::Selected {
                            selected.insert(key(trial.block, trial.partition as u8));
                        }
                    }
                    DecisionEvent::Mode(trial) => assert_cost(trial.cost, lambda_q16),
                    _ => {}
                }
            }
            assert_eq!(committed, decoded, "frame {index}");
            let nodes: BTreeSet<_> = committed.0.union(&leaves).copied().collect();
            assert!(nodes.is_subset(&selected), "frame {index}");
            trials_elsewhere += selected.difference(&nodes).count();
        }
        assert!(trials_elsewhere > 0);
        for outcome in [
            PartitionOutcome::Rejected,
            PartitionOutcome::OverBound,
            PartitionOutcome::Replayed,
            PartitionOutcome::Pruned(PartitionPruning::EarlyExit),
        ] {
            assert!(outcomes.contains(&format!("{outcome:?}")), "{outcome:?}");
        }
    }
}

/// An ordered key of the node `id` with `code`: the partition a split node
/// codes, or the prediction kind of a leaf.
fn key(id: BlockEvidenceId, code: u8) -> (usize, u8, [usize; 4], u8) {
    let rect = id.coded;
    (
        id.tile,
        id.tree as u8,
        [rect.x(), rect.y(), rect.width(), rect.height()],
        code,
    )
}

/// Asserts that `cost` is the documented cost at the multiplier `lambda_q16`.
fn assert_cost(cost: TrialCost, lambda_q16: u64) {
    let rate = u128::from(lambda_q16) * u128::from(cost.rate);
    assert_eq!(cost.cost, (u128::from(cost.distortion) << 24) + rate);
}

/// Three rows of superblocks, in one tile and in two: superblocks and tiles
/// search at the same time, and a budget ends inside a late superblock or
/// inside a frame of a wave.
#[test]
fn a_parallel_search_gives_the_trace_of_one_thread() {
    for tiles_log2 in [0, 1] {
        let mut config = config(192, 192, BitDepth::Eight);
        config.tiles = TileLayout::Uniform {
            cols_log2: tiles_log2,
            rows_log2: 0,
        };
        let mut threaded = config.clone();
        threaded.threads = ThreadCount::from(3);
        let (plain, _) = encode(&config, 4, None);
        let (_, full) = encode(&config, 4, Some(everything(0..u64::MAX)));
        let full = full.unwrap();
        let coded = frames(&full);
        let last_tile = usize::from(tiles_log2);
        let superblocks = coded[0].1.iter().filter(|event| {
            matches!(event, DecisionEvent::Superblock(search) if search.block.tile == last_tile)
        });
        assert!(superblocks.count() >= 3);
        let first = coded[0].1.len() + 1;
        let second = first + coded[1].1.len() + 1;
        let third = second + coded[2].1.len() / 2;
        for max_events in [first - 7, second + 9, third, usize::MAX] {
            let request = limited(0..u64::MAX, max_events, usize::MAX);
            let (packets, trace) = encode(&threaded, 4, Some(request));
            assert_eq!(packets, plain);
            let trace = trace.unwrap();
            let kept = max_events.min(full.events.len());
            assert_eq!(
                trace.events,
                full.events[..kept],
                "{tiles_log2} {max_events}"
            );
            assert_eq!(trace.status == full.status, max_events == usize::MAX);
        }
    }
}

#[test]
fn mode_trials_keep_their_stage_and_what_the_search_did_with_them() {
    let mut config = config(64, 64, BitDepth::Eight);
    config.preset = Preset::new(0).unwrap();
    let request = everything(0..1).with_region(PlaneRect::new(0, 0, 16, 16).unwrap());
    let (packets, trace) = encode(&config, 1, Some(request));
    assert_eq!(packets, encode(&config, 1, None).0);
    let trace = trace.unwrap();
    assert_eq!(trace.status, DecisionTraceStatus::Complete);
    let mut outcomes = BTreeSet::new();
    for event in &trace.events {
        let block = match event {
            DecisionEvent::Mode(trial) => {
                let outcome = trial.outcome.unwrap();
                let ranks = matches!(outcome, ModeOutcome::Advanced | ModeOutcome::Pruned);
                assert_eq!(ranks, trial.stage == ModeStage::Deadzone);
                outcomes.insert(outcome as u8);
                trial.block
            }
            DecisionEvent::Superblock(search) => search.block,
            DecisionEvent::Partition(trial) => trial.block,
            DecisionEvent::Committed(node) => node.block,
            _ => continue,
        };
        assert!(block.coded.x() < 16 && block.coded.y() < 16, "{block:?}");
    }
    assert_eq!(outcomes.len(), 4);
}

#[test]
fn a_frame_coded_again_gives_one_attempt_per_coding() {
    let mut config = config(48, 32, BitDepth::Eight);
    config.rate_control = SMALL_BUFFER;
    let (packets, trace) = encode(&config, 8, Some(everything(0..u64::MAX)));
    let trace = trace.unwrap();
    assert_eq!(trace.status, DecisionTraceStatus::Complete);
    let stats: Vec<_> = packets.iter().flat_map(|packet| &packet.stats).collect();
    let coded = frames(&trace);
    let mut recoded = 0;
    for (at, (frame, events)) in coded.iter().enumerate() {
        let room = frame.buffer_room_bits.unwrap();
        match frame.outcome {
            FrameOutcome::Recoded => {
                recoded += 1;
                assert!(events.is_empty());
                assert!(frame.bits > room);
                let (next, _) = coded[at + 1];
                assert_eq!((next.plan, next.pts), (frame.plan, frame.pts));
                assert_eq!(next.attempt, frame.attempt + 1);
                assert!(next.qindex > frame.qindex);
                assert_eq!(next.buffer_room_bits, Some(room));
            }
            FrameOutcome::Committed(_) => {
                assert!(!events.is_empty());
                let stats = stats[frame.plan.frame_index as usize];
                assert_eq!(u32::from(stats.qp), frame.qindex);
                assert_eq!(stats.bits, frame.bits);
                assert_eq!(stats.pts, frame.pts);
            }
            _ => unreachable!(),
        }
    }
    assert!(recoded > 0);
    assert_eq!(coded.len(), 8 + recoded);
}

#[test]
fn a_budget_stops_the_capture_at_the_same_event() {
    let config = config(96, 64, BitDepth::Eight);
    let (_, full) = encode(&config, 3, Some(everything(0..u64::MAX)));
    let full = full.unwrap();
    let first_frame = frames(&full)[0].1.len() as u64 + 1;
    let size = size_of::<DecisionEvent>();
    for (max_events, max_bytes, kept) in [
        (40, usize::MAX, 40),
        (usize::MAX, 25 * size + size - 1, 25),
        (1, usize::MAX, 1),
    ] {
        let (_, trace) = encode(
            &config,
            3,
            Some(limited(0..u64::MAX, max_events, max_bytes)),
        );
        let trace = trace.unwrap();
        let stop = DecisionTraceStatus::Partial(DecisionTraceStop::Budget);
        assert_eq!(trace.status, stop);
        assert_eq!(trace.events, full.events[..kept]);
        assert_eq!(trace.dropped_events, first_frame - kept as u64);
    }
    for (max_events, max_bytes) in [(0, usize::MAX), (usize::MAX, size - 1)] {
        let (_, trace) = encode(
            &config,
            3,
            Some(limited(0..u64::MAX, max_events, max_bytes)),
        );
        let trace = trace.unwrap();
        let stop = DecisionTraceStatus::Dropped(DecisionTraceStop::Budget);
        assert_eq!((trace.status, trace.events.len()), (stop, 0));
        assert_eq!(trace.dropped_events, first_frame);
    }
    let exact = full.events.len();
    let (_, trace) = encode(&config, 3, Some(limited(0..u64::MAX, exact, usize::MAX)));
    assert_eq!(trace.unwrap(), full);
}

#[test]
fn a_request_selects_frames_by_input_index() {
    let config = config(96, 64, BitDepth::Eight);
    let (packets, trace) = encode(&config, 5, Some(everything(2..4)));
    let trace = trace.unwrap();
    assert_eq!(trace.status, DecisionTraceStatus::Complete);
    let mut displayed: Vec<u64> = frames(&trace)
        .iter()
        .map(|(frame, _)| frame.plan.display_index)
        .collect();
    displayed.sort_unstable();
    assert_eq!(displayed, [2, 3]);
    assert_eq!(packets, encode(&config, 5, None).0);

    let (_, frames_only) = encode(
        &config,
        5,
        Some(DecisionTraceRequest::new(
            0..u64::MAX,
            usize::MAX,
            usize::MAX,
        )),
    );
    let frames_only = frames_only.unwrap();
    assert_eq!(frames_only.status, DecisionTraceStatus::Complete);
    assert_eq!(frames_only.events.len(), 5);

    let (_, none) = encode(&config, 5, Some(everything(5..9)));
    let none = none.unwrap();
    let stop = DecisionTraceStatus::Dropped(DecisionTraceStop::NoFrame);
    assert_eq!((none.status, none.events.len()), (stop, 0));
}

#[test]
fn an_early_take_or_a_cancelled_encoder_gives_a_partial_trace() {
    let config = config(96, 64, BitDepth::Eight);
    let (plain, _) = encode(&config, 5, None);
    let (_, full) = encode(&config, 5, Some(everything(0..u64::MAX)));
    let full = full.unwrap();

    let mut encoder = Encoder::new(config.clone()).unwrap();
    encoder.start_decision_trace(everything(0..u64::MAX));
    for index in 0..5 {
        encoder.send_frame(source(&config, index)).unwrap();
    }
    encoder.flush().unwrap();
    let mut packets = vec![encoder.receive_packet().unwrap().unwrap()];
    let early = encoder.take_decision_trace().unwrap();
    let stop = DecisionTraceStatus::Partial(DecisionTraceStop::Unfinished);
    assert_eq!(early.status, stop);
    assert_eq!(early.events, full.events[..early.events.len()]);
    assert!(encoder.take_decision_trace().is_none());
    encoder.start_decision_trace(everything(0..u64::MAX));
    let at_once = encoder.take_decision_trace().unwrap();
    let stop = DecisionTraceStatus::Dropped(DecisionTraceStop::Unfinished);
    assert_eq!((at_once.status, at_once.events.len()), (stop, 0));
    encoder.start_decision_trace(everything(0..u64::MAX));
    while let Some(packet) = encoder.receive_packet().unwrap() {
        packets.push(packet);
    }
    assert_eq!(packets, plain);
    let late = encoder.take_decision_trace().unwrap();
    assert_eq!(late.status, DecisionTraceStatus::Complete);
    assert_eq!(late.events, full.events[early.events.len()..]);

    let mut encoder = Encoder::new(config.clone()).unwrap();
    encoder.start_decision_trace(everything(0..u64::MAX));
    for index in 0..5 {
        encoder.send_frame(source(&config, index)).unwrap();
    }
    encoder.flush().unwrap();
    assert_eq!(encoder.receive_packet().unwrap().unwrap(), plain[0]);
    encoder.cancel_token().cancel();
    assert!(matches!(encoder.receive_packet(), Err(Error::Cancelled)));
    let failed = encoder.take_decision_trace().unwrap();
    let stop = DecisionTraceStatus::Partial(DecisionTraceStop::EncodeFailed);
    assert_eq!(failed.status, stop);
    assert_eq!(failed.events, full.events[..failed.events.len()]);
}
