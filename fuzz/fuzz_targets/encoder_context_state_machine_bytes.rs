// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_parallel::ThreadCount;
use splot_encode::{
    Context, EncoderConfig, EncoderOperation, EncoderRuntimeConfig, EncoderState, Error,
    FlushStatus, Frame, FrameId, FrameInfo, FramePlaneInput, FramePlanesInput, PlaneRect,
    PlaneSize, ReceivePacketStatus, SendFrameStatus,
};

const MAX_COMMANDS: usize = 64;

fuzz_target!(|data: &[u8]| {
    let runtime = EncoderRuntimeConfig::new(ThreadCount::from(1usize));
    let mut context = Context::new(EncoderConfig::default(), runtime)
        .unwrap_or_else(|err| panic!("single-thread encoder context should construct: {err:?}"));
    let mut next_frame_id = 0_u64;

    exercise_receive_before_input(&mut context);

    for command in data.iter().copied().take(MAX_COMMANDS) {
        match command % 5 {
            0 => {
                send_one(&mut context, next_frame_id);
                next_frame_id = next_frame_id.wrapping_add(1);
            }
            1 => receive_one(&mut context),
            2 => flush_one(&mut context),
            3 => {
                send_one(&mut context, next_frame_id);
                next_frame_id = next_frame_id.wrapping_add(1);
                send_one(&mut context, next_frame_id);
                next_frame_id = next_frame_id.wrapping_add(1);
            }
            _ => {
                flush_one(&mut context);
                receive_one(&mut context);
            }
        }
        assert_invariants(&context);
    }
});

fn exercise_receive_before_input(context: &mut Context) {
    assert_eq!(
        context
            .receive_packet()
            .unwrap_or_else(|err| panic!("initial receive should not fail: {err:?}")),
        ReceivePacketStatus::NeedMoreData
    );
    assert_eq!(context.state(), EncoderState::Accepting);
}

fn send_one(context: &mut Context, frame_id: u64) {
    let y = [0_u8; 4];
    let u = [0_u8; 1];
    let v = [0_u8; 1];
    let frame = frame(frame_id, &y, &u, &v);
    match context.send_frame(&frame) {
        Ok(SendFrameStatus::Accepted {
            queued_frames,
            queue_capacity,
        }) => {
            assert_eq!(context.state(), EncoderState::Accepting);
            assert!(queued_frames <= queue_capacity);
            assert_eq!(queued_frames, context.queued_input_frames());
        }
        Ok(SendFrameStatus::QueueFull {
            queued_frames,
            queue_capacity,
        }) => {
            assert_eq!(context.state(), EncoderState::Accepting);
            assert_eq!(queued_frames, queue_capacity);
            assert_eq!(queued_frames, context.queued_input_frames());
        }
        Err(Error::State {
            operation: EncoderOperation::SendFrame,
            state: EncoderState::Draining | EncoderState::Finished | EncoderState::Failed,
        }) => {}
        Err(err) => panic!("unexpected send_frame result: {err:?}"),
    }
}

fn receive_one(context: &mut Context) {
    match context.receive_packet() {
        Ok(ReceivePacketStatus::NeedMoreData | ReceivePacketStatus::Finished) => {}
        Ok(ReceivePacketStatus::Packet(packet)) => {
            assert!(!packet.data.is_empty());
        }
        Err(Error::State {
            operation: EncoderOperation::ReceivePacket,
            state: EncoderState::Failed,
        }) => {}
        Err(err) => panic!("unexpected receive_packet result: {err:?}"),
    }
}

fn flush_one(context: &mut Context) {
    match context.flush() {
        Ok(FlushStatus::Draining { queued_frames }) => {
            assert_eq!(context.state(), EncoderState::Draining);
            assert_eq!(queued_frames, context.queued_input_frames());
        }
        Ok(FlushStatus::Finished) => {
            assert_eq!(context.state(), EncoderState::Finished);
            assert_eq!(context.queued_input_frames(), 0);
        }
        Err(Error::State {
            operation: EncoderOperation::Flush,
            state: EncoderState::Failed,
        }) => {}
        Err(err) => panic!("unexpected flush result: {err:?}"),
    }
}

fn assert_invariants(context: &Context) {
    assert!(context.queued_input_frames() <= context.input_queue_capacity());
    if context.state() == EncoderState::Finished {
        assert_eq!(context.queued_input_frames(), 0);
    }
}

fn frame<'a>(id: u64, y: &'a [u8], u: &'a [u8], v: &'a [u8]) -> Frame<'a> {
    Frame::from_planes(
        FrameInfo::yuv420_8bit(FrameId::new(id), size(2, 2)),
        FramePlanesInput::yuv(
            FramePlaneInput::new(y, 2, rect(2, 2)),
            FramePlaneInput::new(u, 1, rect(1, 1)),
            FramePlaneInput::new(v, 1, rect(1, 1)),
        ),
    )
    .unwrap_or_else(|err| panic!("fixed fuzz frame should be valid: {err:?}"))
}

fn size(width: usize, height: usize) -> PlaneSize {
    PlaneSize::new(width, height)
        .unwrap_or_else(|err| panic!("fixed nonzero size should be valid: {err:?}"))
}

fn rect(width: usize, height: usize) -> PlaneRect {
    PlaneRect::new(0, 0, width, height)
        .unwrap_or_else(|err| panic!("fixed visible rect should be valid: {err:?}"))
}
