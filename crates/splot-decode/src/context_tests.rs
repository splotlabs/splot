// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

#[test]
fn default_runtime_config_is_auto() {
    assert_eq!(
        DecodeRuntimeConfig::default().thread_count,
        ThreadCount::Auto
    );
}

#[test]
fn context_resolves_fixed_thread_count() {
    let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(4usize))).unwrap();
    assert_eq!(ctx.threads().get(), 4);
}

#[test]
fn requested_threads_round_trips() {
    let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(4usize))).unwrap();
    assert_eq!(ctx.requested_threads(), ThreadCount::from(4usize));
}

#[test]
fn zero_threads_maps_to_auto() {
    let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(0usize))).unwrap();
    assert!(ctx.threads().get() >= 1);
}
