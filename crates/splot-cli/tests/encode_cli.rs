// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end `splot encode` CLI contract tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Output;

fn splot(args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_splot"))
        .args(args)
        .output()
        .expect("failed to run the splot binary")
}

fn assert_accepted_but_unimplemented(args: &[&str]) {
    let out = splot(args);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert_eq!(out.status.code(), Some(1), "stderr was: {stderr}");
    assert!(
        stderr.contains("not yet implemented"),
        "stderr was: {stderr}"
    );
}

#[test]
fn encode_threads_fixed_is_accepted_but_unimplemented() {
    assert_accepted_but_unimplemented(&["encode", "--threads", "4", "in.y4m", "-o", "out.av2"]);
}

#[test]
fn encode_threads_auto_is_accepted() {
    assert_accepted_but_unimplemented(&["encode", "--threads", "auto", "in.y4m", "-o", "out.av2"]);
}

#[test]
fn encode_threads_zero_is_accepted_as_auto() {
    assert_accepted_but_unimplemented(&["encode", "--threads", "0", "in.y4m", "-o", "out.av2"]);
}

#[test]
fn encode_threads_invalid_is_usage_error() {
    let out = splot(&["encode", "--threads", "nope", "in.y4m", "-o", "out.av2"]);

    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("not yet implemented"),
        "stderr was: {stderr}"
    );
}

#[test]
fn encode_default_threads_is_accepted() {
    assert_accepted_but_unimplemented(&["encode", "in.y4m", "-o", "out.av2"]);
}

#[test]
fn encode_speed_supported_value_is_accepted_but_unimplemented() {
    assert_accepted_but_unimplemented(&["encode", "--speed", "10", "in.y4m", "-o", "out.av2"]);
}

#[test]
fn encode_speed_unsupported_value_is_usage_error() {
    let out = splot(&["encode", "--speed", "11", "in.y4m", "-o", "out.av2"]);

    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("encoder speed preset 11 is outside the supported range 0..=10"),
        "stderr was: {stderr}"
    );
    assert!(
        !stderr.contains("not yet implemented"),
        "stderr was: {stderr}"
    );
}
