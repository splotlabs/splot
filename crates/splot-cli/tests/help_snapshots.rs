// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Snapshots of validate, inspect and explain help (CONF-CLI-SNAPSHOT-COVERAGE).
//! Top-level help is excluded because decoder wording changes independently.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

/// Runs `splot <subcommand> --help` and returns its stdout, asserting a clean exit.
fn help(subcommand: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_splot"))
        .args([subcommand, "--help"])
        .output()
        .expect("failed to run the splot binary");
    assert!(
        out.status.success(),
        "`splot {subcommand} --help` exited with {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8(out.stdout).expect("help stdout is valid UTF-8")
}

#[test]
fn validate_help() {
    insta::assert_snapshot!("validate_help", help("validate"));
}

#[test]
fn inspect_help() {
    insta::assert_snapshot!("inspect_help", help("inspect"));
}

#[test]
fn explain_help() {
    insta::assert_snapshot!("explain_help", help("explain"));
}
