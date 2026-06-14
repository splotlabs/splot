// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Backward-compatibility snapshots of the validator-stream subcommands'
//! `--help` — `validate`, `inspect`, and `explain` (CONF-CLI-SNAPSHOT-COVERAGE).
//!
//! `--help` is rendered from the static clap command definition, so it is fully
//! deterministic and carries no version string. Freezing it as an `insta` golden
//! makes every change to those subcommands' argument surface — a new flag, a
//! renamed flag, a changed help string, a reordered option — show up as a
//! reviewable snapshot diff (`cargo insta review`). This is the public-surface
//! tripwire for the validator productization work: additive flags update these
//! snapshots intentionally; an accidental or breaking surface change is caught
//! here. The top-level `splot --help` is deliberately NOT snapshotted so this
//! validator-stream test stays decoupled from the `decode`/`encode` subcommand
//! wording the decoder/encoder streams own.

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
