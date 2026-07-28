// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Retention bounds for the filter-source buffer caches.

#![allow(clippy::expect_used)]

use super::{
    MAX_RETAINED_STRIPE_SAMPLES, MAX_RETAINED_WINDOW_SAMPLES, WINDOW_SAMPLE_BUFFERS,
    lock_stripe_sample_buffers, recycle_stripe_sample_buffer, recycle_window_buffer,
};

fn oversized(samples: usize) -> Vec<u16> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(samples)
        .expect("an oversized sample buffer");
    buffer
}

#[test]
fn the_deblock_window_cache_never_retains_an_oversized_buffer() {
    recycle_window_buffer(oversized(MAX_RETAINED_WINDOW_SAMPLES + 1));

    let buffers = WINDOW_SAMPLE_BUFFERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        buffers.iter().all(|buffer| buffer
            .downcast_ref::<Vec<u16>>()
            .is_none_or(|buffer| buffer.capacity() <= MAX_RETAINED_WINDOW_SAMPLES)),
        "the window cache is bounded by a constant, not by the widest frame the process decoded"
    );
}

#[test]
fn the_stripe_cache_never_retains_an_oversized_buffer() {
    recycle_stripe_sample_buffer(oversized(MAX_RETAINED_STRIPE_SAMPLES + 1));

    let buffers = lock_stripe_sample_buffers();
    assert!(
        buffers
            .iter()
            .all(|buffer| buffer.capacity() <= MAX_RETAINED_STRIPE_SAMPLES),
        "the stripe cache is bounded by a constant"
    );
}
