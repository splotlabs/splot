// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Selection and retention policies for the filter-source buffer caches.

#![allow(clippy::expect_used)]

use super::{StripePlane, select_buffer_index};
use splot_recon::PlaneRect;
use std::any::Any;

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
fn stripe_rect_mut_rejects_a_rectangle_overhanging_the_row() {
    let mut stripe = StripePlane::from_samples(4, 2, 0, vec![0; 8]).expect("a valid stripe");
    let rect = PlaneRect::new(3, 0, 2, 1).expect("a valid rectangle");

    assert!(stripe.rect_mut(rect).is_none());
}
