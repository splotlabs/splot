// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use core::cell::RefCell;

use splot_parallel::ThreadCount;
use splot_recon::{BitDepth, PixelFormat};

use super::*;
use crate::config::Config;

/// A smooth texture panned right by `pan` luma samples per frame, with
/// uniform noise of up to `amplitude` 8-bit steps from `seed` (more in V
/// than in U, more in U than in Y).
fn frame(config: &Config, index: u32, pan: f64, amplitude: u32, seed: u32) -> SourceFrame {
    let sizes = config.plane_sizes().unwrap();
    let scale = 1 << (config.bit_depth.bits() - 8);
    let mut state = seed.wrapping_mul(2_654_435_761) | 1;
    let planes = core::array::from_fn(|plane| {
        let Some(&size) = sizes.get(plane) else {
            return Vec::new();
        };
        let (sx, sy) = match plane {
            0 => (0, 0),
            _ => (config.chroma.subsampling_x(), config.chroma.subsampling_y()),
        };
        let spread = amplitude * (2 + plane as u32) / 2;
        (0..size.width() * size.height())
            .map(|i| {
                let x = ((i % size.width()) << sx) as f64 - pan * f64::from(index);
                let y = ((i / size.width()) << sy) as f64;
                let v = 120.0
                    + 50.0 * (0.21 * x + 0.05 * y + plane as f64).sin()
                    + 40.0 * (0.13 * y - 0.04 * x).cos()
                    + 25.0 * (0.0011 * x * y).sin();
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let noise = if spread == 0 {
                    0.0
                } else {
                    f64::from(state % (2 * spread + 1)) - f64::from(spread)
                };
                ((v + noise).clamp(0.0, 255.0) * f64::from(scale)).round() as u16
            })
            .collect()
    });
    SourceFrame::new(config, u64::from(index), planes).unwrap()
}

/// A plan of `display` as a hidden base frame or a key frame.
fn plan(display: u64, frame_type: FrameType) -> GopFrame {
    GopFrame {
        frame_index: display,
        display_index: display,
        frame_type,
        order_hint: display as u32,
        show_frame: frame_type == FrameType::Key,
        layer: 0,
        refresh_frame_flags: 1,
        ref_slots: 1,
        primary_slot: None,
        opens_temporal_unit: true,
    }
}

/// A clip of `count` frames analysed by a lookahead on `pool`.
struct Clip {
    frames: Vec<SourceFrame>,
    lookahead: Lookahead,
}

fn clip(
    config: &Config,
    count: u32,
    make: impl Fn(u32) -> SourceFrame,
    pool: Option<WorkerPool>,
) -> Clip {
    let mut lookahead = Lookahead::new(config, config.preset.params(), pool);
    let frames: Vec<SourceFrame> = (0..count).map(make).collect();
    for (index, frame) in frames.iter().enumerate() {
        lookahead.push(frame, index as u64);
    }
    Clip { frames, lookahead }
}

impl Clip {
    fn filter(
        &self,
        plan: &GopFrame,
        qindex: u16,
        frames: u8,
        pool: Option<&WorkerPool>,
    ) -> Option<SourceFrame> {
        let sources = |display: u64| self.frames.get(usize::try_from(display).ok()?);
        filter_frame(
            &self.lookahead,
            &sources,
            plan,
            qindex,
            [(frames, 100); 2],
            subsampling(&self.frames[0]),
            pool,
        )
    }
}

fn subsampling(frame: &SourceFrame) -> [u8; 2] {
    let sizes = frame.plane_sizes();
    match sizes.get(1) {
        Some(chroma) => [
            u8::from(chroma.width() < sizes[0].width()),
            u8::from(chroma.height() < sizes[0].height()),
        ],
        None => [1, 1],
    }
}

fn mse(a: &[u16], b: &[u16]) -> f64 {
    let sum: f64 = a
        .iter()
        .zip(b)
        .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
        .sum();
    sum / a.len() as f64
}

/// A still and three panning noisy clips, in every chroma format and both
/// bit depths, with blocks cut by the frame edges: the filtered base frame
/// is much closer to the noise-free frame than its source in every plane,
/// so the weights follow the motion.
#[test]
fn filtering_removes_noise_along_the_motion() {
    for (pan, bit_depth, chroma) in [
        (0.0, BitDepth::Eight, PixelFormat::Yuv420),
        (4.0, BitDepth::Ten, PixelFormat::Yuv422),
        (3.0, BitDepth::Eight, PixelFormat::Yuv444),
        (6.0, BitDepth::Eight, PixelFormat::Monochrome),
    ] {
        let mut config = Config::new(136, 72);
        config.bit_depth = bit_depth;
        config.chroma = chroma;
        let noisy = clip(&config, 5, |i| frame(&config, i, pan, 12, i + 1), None);
        let filtered = noisy
            .filter(&plan(2, FrameType::Inter), 255, 2, None)
            .unwrap();
        let clean = frame(&config, 2, pan, 0, 0);
        for plane in 0..config.plane_sizes().unwrap().len() {
            let before = mse(&noisy.frames[2].planes()[plane], &clean.planes()[plane]);
            let after = mse(&filtered.planes()[plane], &clean.planes()[plane]);
            assert!(
                after < 0.5 * before,
                "{pan} {chroma:?} plane {plane}: {after} vs {before}"
            );
        }
        assert_eq!(filtered.plane_sizes(), noisy.frames[2].plane_sizes());
    }
}

/// The filter reads the nearest frames first, both sides of a base frame
/// and only later frames of a key frame; a frame without noise reads one
/// neighbour per side.
#[test]
fn neighbours_follow_the_frame_kind_and_the_noise() {
    let config = Config::new(96, 64);
    for (amplitude, kind, display, read) in [
        (12, FrameType::Inter, 3, vec![3, 4, 2, 5, 1, 6, 0]),
        (12, FrameType::Key, 3, vec![3, 4, 5, 6]),
        (0, FrameType::Inter, 3, vec![3, 4, 2]),
        (12, FrameType::Inter, 6, vec![6, 7, 5, 4, 3]),
    ] {
        let noisy = clip(
            &config,
            7,
            |i| frame(&config, i, 2.0, amplitude, i + 3),
            None,
        );
        let seen = RefCell::new(Vec::new());
        let sources = |display: u64| {
            seen.borrow_mut().push(display);
            noisy.frames.get(usize::try_from(display).ok()?)
        };
        filter_frame(
            &noisy.lookahead,
            &sources,
            &plan(display, kind),
            250,
            [(3, 100); 2],
            [1, 1],
            None,
        );
        assert_eq!(*seen.borrow(), read, "{amplitude} {kind:?} {display}");
    }
}

/// Shown inter frames and upper-layer frames are not filtered, nor is any
/// frame when the frame count or the strength is zero.
#[test]
fn only_key_and_hidden_base_frames_are_filtered() {
    let config = Config::new(64, 64);
    let noisy = clip(&config, 3, |i| frame(&config, i, 0.0, 12, i + 5), None);
    let shown = GopFrame {
        show_frame: true,
        ..plan(1, FrameType::Inter)
    };
    let upper = GopFrame {
        layer: 1,
        ..plan(1, FrameType::Inter)
    };
    assert!(noisy.filter(&shown, 200, 1, None).is_none());
    assert!(noisy.filter(&upper, 200, 1, None).is_none());
    assert!(
        noisy
            .filter(&plan(1, FrameType::Inter), 200, 0, None)
            .is_none()
    );
    let sources = |display: u64| noisy.frames.get(display as usize);
    let base = plan(1, FrameType::Inter);
    assert!(
        filter_frame(
            &noisy.lookahead,
            &sources,
            &base,
            200,
            [(1, 100), (1, 0)],
            [1, 1],
            None
        )
        .is_none()
    );
    assert!(noisy.filter(&base, 200, 1, None).is_some());
    assert!(
        noisy
            .filter(&plan(0, FrameType::Key), 200, 1, None)
            .is_some()
    );
}

/// A filtered frame further from its source than the qindex allows is not
/// used: at qindex 40 noise of 8 steps is kept, at 200 it is removed.
#[test]
fn a_filter_that_moves_the_frame_too_far_is_not_used() {
    let config = Config::new(96, 64);
    let noisy = clip(&config, 3, |i| frame(&config, i, 0.0, 8, i + 9), None);
    assert!(
        noisy
            .filter(&plan(1, FrameType::Inter), 40, 1, None)
            .is_none()
    );
    assert!(
        noisy
            .filter(&plan(1, FrameType::Inter), 200, 1, None)
            .is_some()
    );
}

/// The filtered frame does not depend on the worker pool of the filter or of
/// the lookahead's search.
#[test]
fn worker_pools_filter_alike() {
    let mut config = Config::new(200, 136);
    config.bit_depth = BitDepth::Ten;
    let make = |i| frame(&config, i, 3.0, 9, i + 11);
    let serial = clip(&config, 5, make, None).filter(&plan(2, FrameType::Inter), 180, 2, None);
    assert!(serial.is_some());
    for threads in [1, 2, 8] {
        let pool = WorkerPool::new(ThreadCount::from(threads)).unwrap();
        let parallel = clip(&config, 5, make, Some(pool.clone()));
        let filtered = parallel.filter(&plan(2, FrameType::Inter), 180, 2, Some(&pool));
        assert!(filtered == serial, "{threads} threads");
    }
}

/// The noise level is zero for identical frames and grows with the noise.
#[test]
fn the_noise_level_grows_with_the_noise() {
    let config = Config::new(128, 64);
    let levels: Vec<u32> = [0, 3, 12]
        .into_iter()
        .map(|amplitude| {
            let noisy = clip(
                &config,
                2,
                |i| frame(&config, i, 0.0, amplitude, i + 1),
                None,
            );
            let full = [0, 1].map(|i| hme::full_level(&noisy.frames[i]));
            noise_level(
                &noisy
                    .lookahead
                    .full_motion((0, 1), (&full[0], &full[1]))
                    .unwrap(),
            )
        })
        .collect();
    assert!(
        levels[0] == 0 && levels[1] > 0 && levels[2] > CLEAN_NOISE,
        "{levels:?}"
    );
    assert!(levels[2] > 4 * levels[1], "{levels:?}");
}
