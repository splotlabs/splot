// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]
use super::*;
use splot_parallel::{ThreadCount, WorkerPool};

fn serial_write(plane: &mut [u16], stride: usize, runs: &[RectRun<u16>]) {
    for (rect, samples, row_stride) in runs {
        for row in 0..rect.height() {
            let src = &samples[row * row_stride..row * row_stride + rect.width()];
            let dst_start = (rect.y() + row) * stride + rect.x();
            plane[dst_start..dst_start + rect.width()].copy_from_slice(src);
        }
    }
}

fn run(x: usize, y: usize, w: usize, h: usize, tag: u16) -> RectRun<u16> {
    let rect = PlaneRect::new(x, y, w, h).unwrap();
    let samples: Vec<u16> = (0..w * h).map(|i| tag.wrapping_add(i as u16)).collect();
    (rect, samples, w)
}

fn pool() -> WorkerPool {
    WorkerPool::new(ThreadCount::Fixed(4.try_into().unwrap())).unwrap()
}

#[test]
fn parallel_banded_write_equals_serial_in_order() {
    let stride = 32usize;
    let height = 40usize;
    let runs = vec![
        run(0, 0, 8, 4, 100),
        run(8, 0, 8, 4, 200),
        run(16, 0, 16, 8, 300), // taller: extends group_bottom to 8
        run(0, 4, 8, 4, 400),   // y=4 < 8: same band
        run(0, 8, 32, 2, 500),  // y=8 >= 8: new band
        run(0, 10, 32, 30, 600),
    ];
    let mut par = vec![0u16; stride * height];
    let mut ser = vec![0u16; stride * height];
    let got = pool().install(|| publish_rect_runs_parallel(&mut par, stride, &runs));
    assert_eq!(got, Some(()), "valid raster-order input must publish");
    serial_write(&mut ser, stride, &runs);
    assert_eq!(par, ser, "parallel banded write must equal serial in-order");
}

#[test]
fn none_on_single_worker_pool_leaves_plane_untouched() {
    let stride = 8usize;
    let runs = vec![run(0, 0, 4, 2, 7)];
    let mut plane = vec![0u16; stride * 4];
    let single = WorkerPool::new(ThreadCount::Fixed(1.try_into().unwrap())).unwrap();
    let got = single.install(|| publish_rect_runs_parallel(&mut plane, stride, &runs));
    assert_eq!(got, None);
    assert!(plane.iter().all(|&v| v == 0), "None path must not write");
}

#[test]
fn partial_write_then_none_plus_serial_fallback_equals_serial() {
    let stride = 16usize;
    let height = 24usize;
    let mut runs = vec![run(0, 0, 8, 4, 111), run(0, 8, 8, 4, 222)];
    runs.push(run(0, height, 8, 4, 333));
    let mut par = vec![0u16; stride * height];
    let got = pool().install(|| publish_rect_runs_parallel(&mut par, stride, &runs[..]));
    let in_range = &runs[..2];
    let mut fallback = par.clone();
    if got.is_none() {
        serial_write(&mut fallback, stride, in_range);
    }
    let mut fresh = vec![0u16; stride * height];
    serial_write(&mut fresh, stride, in_range);
    assert_eq!(got, None, "out-of-plane rect must yield None");
    assert_eq!(
        fallback, fresh,
        "serial fallback after a partial parallel write must equal pure serial"
    );
}

#[test]
fn writes_rect_at_band_origin_offset() {
    let mut band = vec![0u16; 4 * 8];
    let rect = PlaneRect::new(2, 5, 3, 2).unwrap();
    let samples = [1u16, 2, 3, 4, 5, 6];
    write_rect_into_band(&mut band, 8, 4, rect, &samples, 3).unwrap();
    assert_eq!(&band[8 + 2..8 + 5], &[1, 2, 3]);
    assert_eq!(&band[2 * 8 + 2..2 * 8 + 5], &[4, 5, 6]);
}

#[test]
fn rejects_rect_above_band() {
    let mut band = vec![0u16; 8];
    let rect = PlaneRect::new(0, 1, 2, 1).unwrap();
    assert!(write_rect_into_band(&mut band, 8, 2, rect, &[1u16, 2], 2).is_none());
    assert!(band.iter().all(|&value| value == 0));
}

#[test]
fn rejects_rect_below_band() {
    let mut band = vec![0u16; 8];
    let rect = PlaneRect::new(0, 3, 2, 1).unwrap();
    assert!(write_rect_into_band(&mut band, 8, 2, rect, &[1u16, 2], 2).is_none());
}

#[test]
fn rejects_short_sample_buffer() {
    let mut band = vec![0u16; 16];
    let rect = PlaneRect::new(0, 0, 4, 2).unwrap();
    assert!(write_rect_into_band(&mut band, 8, 0, rect, &[1u16; 7], 4).is_none());
}
