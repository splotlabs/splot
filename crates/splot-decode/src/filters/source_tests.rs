// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Selection and retention policies for the filter-source buffer caches.

#![allow(clippy::expect_used)]

use super::{
    MAX_RETAINED_STRIPE_BUFFERS, MAX_RETAINED_WINDOW_BUFFERS, StripePlane, recycle_buffer,
    select_buffer_index,
};
use splot_recon::PlaneRect;
use std::any::Any;

const EIGHT_K_LUMA_WINDOW_SAMPLES: usize = 737_280;

fn oversized(samples: usize) -> Vec<u16> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(samples)
        .expect("an oversized sample buffer");
    buffer
}

#[test]
fn window_cache_selection_matches_type_and_capacity() {
    let buffers: Vec<Box<dyn Any + Send>> = vec![
        Box::new(Vec::<u8>::with_capacity(96)),
        Box::new(Vec::<u16>::with_capacity(32)),
        Box::new(Vec::<u16>::with_capacity(128)),
        Box::new(Vec::<u16>::with_capacity(256)),
    ];
    let capacities = || {
        buffers.iter().enumerate().filter_map(|(index, buffer)| {
            buffer
                .downcast_ref::<Vec<u16>>()
                .map(|buffer| (index, buffer.capacity()))
        })
    };

    assert_eq!(select_buffer_index(capacities(), 96, false), Some(2));
    assert_eq!(select_buffer_index(capacities(), 512, false), None);
    assert_eq!(select_buffer_index(capacities(), 512, true), Some(3));
}

#[test]
fn stripe_cache_selection_uses_fresh_storage_until_full() {
    let capacities = [(0, 32), (1, 128), (2, 256)];

    assert_eq!(select_buffer_index(capacities, 96, false), Some(1));
    assert_eq!(select_buffer_index(capacities, 512, false), None);
    assert_eq!(select_buffer_index(capacities, 512, true), Some(2));
}

#[test]
fn caches_retain_large_frame_buffers_under_their_count_caps() {
    let stripe = oversized(EIGHT_K_LUMA_WINDOW_SAMPLES);
    let stripe_capacity = stripe.capacity();
    let mut stripe_buffers = Vec::new();
    recycle_buffer(
        &mut stripe_buffers,
        stripe,
        stripe_capacity,
        MAX_RETAINED_STRIPE_BUFFERS,
    );
    assert_eq!(stripe_buffers.len(), 1);

    let window = oversized(EIGHT_K_LUMA_WINDOW_SAMPLES);
    let window_capacity = window.capacity();
    let mut window_buffers: Vec<Box<dyn Any + Send>> = Vec::new();
    recycle_buffer(
        &mut window_buffers,
        Box::new(window),
        window_capacity,
        MAX_RETAINED_WINDOW_BUFFERS,
    );
    assert_eq!(window_buffers.len(), 1);
}

#[test]
fn stripe_rect_mut_rejects_a_rectangle_overhanging_the_row() {
    let mut stripe = StripePlane::from_samples(4, 2, 0, vec![0; 8]).expect("a valid stripe");
    let rect = PlaneRect::new(3, 0, 2, 1).expect("a valid rectangle");

    assert!(stripe.rect_mut(rect).is_none());
}
