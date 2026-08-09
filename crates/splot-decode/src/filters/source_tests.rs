// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Selection and retention policies for the filter-source buffer caches.

#![allow(clippy::expect_used)]

use super::{
    MAX_RETAINED_STRIPE_BUFFERS, MAX_RETAINED_WINDOW_BUFFERS, StripePlane, WINDOW_SAMPLE_BUFFERS,
    lock_stripe_sample_buffers, recycle_stripe_sample_buffer, recycle_window_buffer,
    take_stripe_sample_buffer, take_window_buffer,
};
use splot_recon::PlaneRect;

const EIGHT_K_LUMA_WINDOW_SAMPLES: usize = 737_280;

fn oversized(samples: usize) -> Vec<u16> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(samples)
        .expect("an oversized sample buffer");
    buffer
}

#[test]
fn window_cache_selects_by_capacity_and_retains_large_frame_buffers() {
    let small = Vec::<u16>::with_capacity(32);
    let fitting = Vec::<u16>::with_capacity(128);
    let fitting_ptr = fitting.as_ptr();
    let large = Vec::<u16>::with_capacity(256);
    {
        let mut buffers = WINDOW_SAMPLE_BUFFERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        buffers.clear();
        buffers.push(Box::new(small));
        buffers.push(Box::new(fitting));
        buffers.push(Box::new(large));
    }

    let selected = take_window_buffer::<u16>(96).expect("a fitting window buffer");
    assert!(std::ptr::eq(selected.as_ptr(), fitting_ptr));

    let cached = Vec::<u16>::with_capacity(64);
    let cached_ptr = cached.as_ptr();
    {
        let mut buffers = WINDOW_SAMPLE_BUFFERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        buffers.clear();
        buffers.push(Box::new(cached));
    }
    let fresh = take_window_buffer::<u16>(96).expect("a fresh window buffer");
    assert!(!std::ptr::eq(fresh.as_ptr(), cached_ptr));
    assert!(
        WINDOW_SAMPLE_BUFFERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|buffer| buffer
                .downcast_ref::<Vec<u16>>()
                .is_some_and(|buffer| std::ptr::eq(buffer.as_ptr(), cached_ptr))),
        "an undersized buffer must remain cached while the pool has room"
    );

    let fallback = Vec::<u16>::with_capacity(64);
    {
        let mut buffers = WINDOW_SAMPLE_BUFFERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        buffers.clear();
        buffers.push(Box::new(fallback));
        buffers.extend(
            (1..MAX_RETAINED_WINDOW_BUFFERS)
                .map(|_| Box::new(Vec::<u8>::with_capacity(1)) as Box<dyn std::any::Any + Send>),
        );
    }

    let selected = take_window_buffer::<u16>(96).expect("an undersized fallback window buffer");
    assert!(selected.capacity() >= 96);
    assert!(
        WINDOW_SAMPLE_BUFFERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .all(|buffer| buffer.downcast_ref::<Vec<u16>>().is_none()),
        "the only matching fallback must leave the full pool"
    );

    let oversized = oversized(EIGHT_K_LUMA_WINDOW_SAMPLES);
    let oversized_ptr = oversized.as_ptr();
    WINDOW_SAMPLE_BUFFERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    recycle_window_buffer(oversized);

    let mut buffers = WINDOW_SAMPLE_BUFFERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        buffers.iter().any(|buffer| buffer
            .downcast_ref::<Vec<u16>>()
            .is_some_and(|buffer| std::ptr::eq(buffer.as_ptr(), oversized_ptr))),
        "the window cache must retain an 8K luma allocation"
    );
    buffers.clear();
}

#[test]
fn stripe_cache_selects_by_capacity_and_retains_large_frame_buffers() {
    let small = Vec::<u16>::with_capacity(32);
    let fitting = Vec::<u16>::with_capacity(128);
    let fitting_ptr = fitting.as_ptr();
    let large = Vec::<u16>::with_capacity(256);
    {
        let mut buffers = lock_stripe_sample_buffers();
        buffers.clear();
        buffers.extend([small, fitting, large]);
    }

    let selected = take_stripe_sample_buffer(96).expect("a fitting stripe buffer");
    assert!(std::ptr::eq(selected.as_ptr(), fitting_ptr));

    let cached = Vec::<u16>::with_capacity(64);
    let cached_ptr = cached.as_ptr();
    {
        let mut buffers = lock_stripe_sample_buffers();
        buffers.clear();
        buffers.push(cached);
    }
    let fresh = take_stripe_sample_buffer(96).expect("a fresh stripe buffer");
    assert!(!std::ptr::eq(fresh.as_ptr(), cached_ptr));
    assert!(
        lock_stripe_sample_buffers()
            .iter()
            .any(|buffer| std::ptr::eq(buffer.as_ptr(), cached_ptr)),
        "an undersized buffer must remain cached while the pool has room"
    );

    let fallback = Vec::<u16>::with_capacity(64);
    {
        let mut buffers = lock_stripe_sample_buffers();
        buffers.clear();
        buffers.push(fallback);
        buffers.extend((1..MAX_RETAINED_STRIPE_BUFFERS).map(|_| Vec::with_capacity(1)));
    }

    let selected = take_stripe_sample_buffer(96).expect("an undersized fallback stripe buffer");
    assert!(selected.capacity() >= 96);
    assert!(
        lock_stripe_sample_buffers()
            .iter()
            .all(|buffer| buffer.capacity() == 1),
        "the largest fallback must leave the full pool"
    );

    let oversized = oversized(EIGHT_K_LUMA_WINDOW_SAMPLES);
    let oversized_ptr = oversized.as_ptr();
    lock_stripe_sample_buffers().clear();
    recycle_stripe_sample_buffer(oversized);

    let mut buffers = lock_stripe_sample_buffers();
    assert!(
        buffers
            .iter()
            .any(|buffer| std::ptr::eq(buffer.as_ptr(), oversized_ptr)),
        "the stripe cache must retain an 8K luma allocation"
    );
    buffers.clear();
}

#[test]
fn stripe_rect_mut_rejects_a_rectangle_overhanging_the_row() {
    let mut stripe = StripePlane::from_samples(4, 2, 0, vec![0; 8]).expect("a valid stripe");
    let rect = PlaneRect::new(3, 0, 2, 1).expect("a valid rectangle");

    assert!(stripe.rect_mut(rect).is_none());
}
