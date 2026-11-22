// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! [`CancelToken`] through the [`Encoder`], at 8 and 10 bits.

use std::sync::atomic::AtomicUsize;

use super::*;
use crate::{BitDepth, Config, Encoder, Preset, SourceFrame, ThreadCount};

const DEPTHS: [BitDepth; 2] = [BitDepth::Eight, BitDepth::Ten];

fn config(bit_depth: BitDepth, threads: usize) -> Config {
    let mut config = Config::new(96, 64);
    config.bit_depth = bit_depth;
    config.preset = Preset::new(9).unwrap();
    config.threads = ThreadCount::from_count_or_auto(threads);
    config
}

fn frame(config: &Config, pts: u64) -> SourceFrame {
    crate::lookahead::tests::frame(config, pts, pts as usize, 0)
}

/// An encoder of `config` with `frames` frames sent.
fn fed(config: &Config, frames: u64) -> Encoder {
    let mut encoder = Encoder::new(config.clone()).unwrap();
    for pts in 0..frames {
        encoder.send_frame(frame(config, pts)).unwrap();
    }
    encoder
}

/// The packets `encoder` returns before its first error, and that error.
fn drain(encoder: &mut Encoder) -> (Vec<u8>, Option<Error>) {
    let mut bytes = Vec::new();
    loop {
        match encoder.receive_packet() {
            Ok(Some(packet)) => bytes.extend(packet.data),
            Ok(None) => return (bytes, None),
            Err(error) => return (bytes, Some(error)),
        }
    }
}

fn assert_stopped(encoder: &mut Encoder, config: &Config) {
    assert!(matches!(encoder.receive_packet(), Err(Error::Cancelled)));
    assert!(matches!(
        encoder.send_frame(frame(config, 99)),
        Err(Error::Cancelled)
    ));
    assert!(matches!(encoder.flush(), Err(Error::Cancelled)));
}

#[test]
fn a_cancel_before_the_first_frame_or_in_the_lookahead_stops_every_call() {
    for bit_depth in DEPTHS {
        let config = config(bit_depth, 1);
        for sent in [0, 3] {
            let mut encoder = fed(&config, sent);
            assert!(matches!(encoder.receive_packet(), Ok(None)));
            let token = encoder.cancel_token();
            assert!(!token.is_cancelled());
            token.cancel();
            assert!(token.is_cancelled() && encoder.cancel_token().is_cancelled());
            assert_stopped(&mut encoder, &config);
        }
    }
}

/// Cancels from the stats sink of the first frame coded after the key frame,
/// inside the `receive_packet` that codes a mini-GOP of `frames - 1` frames:
/// the call codes no frame of a later wave.
fn cancel_in_mini_gop(bit_depth: BitDepth, frames: u64, flush: bool) {
    let config = config(bit_depth, 2);
    let mut encoder = fed(&config, frames);
    if flush {
        encoder.flush().unwrap();
    }
    let token = encoder.cancel_token();
    let coded = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&coded);
    encoder.set_stats_sink(Box::new(move |stats| {
        if stats.frame_index > 0 {
            token.cancel();
        }
        coded.fetch_add(1, Ordering::Relaxed);
    }));
    let (bytes, error) = drain(&mut encoder);
    assert!(matches!(error, Some(Error::Cancelled)), "{error:?}");
    assert!(
        !bytes.is_empty(),
        "the key frame came out before the cancel"
    );
    assert_eq!(seen.load(Ordering::Relaxed), 2, "{bit_depth:?}");
    assert_stopped(&mut encoder, &config);
}

#[test]
fn a_cancel_inside_a_mini_gop_stops_it() {
    for bit_depth in DEPTHS {
        cancel_in_mini_gop(bit_depth, 9, false);
    }
}

#[test]
fn a_cancel_while_flushing_stops_the_last_mini_gop() {
    for bit_depth in DEPTHS {
        cancel_in_mini_gop(bit_depth, 6, true);
    }
}

/// Another thread cancels while `receive_packet` searches blocks on the
/// worker pool; the call returns and the encoder drops. The other thread
/// waits for the key frame's statistics, so the cancel lands in the call
/// that codes the 8 frames after it.
#[test]
fn a_cancel_from_another_thread_stops_a_running_call() {
    for bit_depth in DEPTHS {
        let mut config = config(bit_depth, 4);
        (config.width, config.height) = (192, 128);
        let mut encoder = fed(&config, 9);
        let token = encoder.cancel_token();
        let coded = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&coded);
        encoder.set_stats_sink(Box::new(move |_| {
            coded.fetch_add(1, Ordering::Relaxed);
        }));
        let waiting = Arc::clone(&seen);
        let canceller = splot_parallel::spawn_thread("canceller", move || {
            while waiting.load(Ordering::Relaxed) == 0 {
                std::thread::yield_now();
            }
            token.cancel();
        })
        .unwrap();
        let (_, error) = drain(&mut encoder);
        assert!(matches!(error, Some(Error::Cancelled)), "{error:?}");
        canceller.join().unwrap();
        assert_eq!(seen.load(Ordering::Relaxed), 1, "{bit_depth:?}");
        assert_stopped(&mut encoder, &config);
    }
}

#[test]
fn a_token_that_is_never_cancelled_changes_no_byte() {
    for bit_depth in DEPTHS {
        let config = config(bit_depth, 2);
        let stream = |hold: bool| {
            let mut encoder = fed(&config, 10);
            let token = hold.then(|| encoder.cancel_token());
            encoder.flush().unwrap();
            let (bytes, error) = drain(&mut encoder);
            assert!(error.is_none() && token.is_none_or(|token| !token.is_cancelled()));
            bytes
        };
        let plain = stream(false);
        assert!(!plain.is_empty() && plain == stream(true), "{bit_depth:?}");
    }
}
