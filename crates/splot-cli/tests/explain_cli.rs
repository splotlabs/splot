// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot explain` end-to-end tests (CLI-VALIDATE-EXPLAIN): golden snapshots of a
//! describe (text + JSON) plus behavioral assertions for `--list`, unknown ids, and
//! the missing-argument path.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::{Command, Output};

const KNOWN_ID: &str = "obu-header/global-xlayer-required";

fn explain(args: &[&str]) -> Output {
    let mut full = vec!["explain"];
    full.extend_from_slice(args);
    Command::new(env!("CARGO_BIN_EXE_splot"))
        .args(&full)
        .output()
        .expect("failed to run the splot binary")
}

#[test]
fn explain_describe_text() {
    let out = explain(&[KNOWN_ID]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    insta::assert_snapshot!("explain_describe_text", stdout);
}

#[test]
fn explain_describe_json() {
    let out = explain(&[KNOWN_ID, "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    insta::assert_snapshot!("explain_describe_json", stdout);
}

#[test]
fn explain_unknown_id_exits_two_with_hint() {
    let out = explain(&["obu-header/does-not-exist"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unknown rule id"), "stderr: {stderr}");
    assert!(stderr.contains("did you mean"), "stderr: {stderr}");
    assert!(stderr.contains("obu-header/"), "stderr: {stderr}");
}

#[test]
fn explain_missing_argument_exits_two() {
    let out = explain(&[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--list"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn explain_list_is_sorted_and_substantial() {
    let out = explain(&["--list"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let ids: Vec<&str> = stdout.lines().collect();
    assert!(ids.len() >= 200, "expected >= 200 ids, got {}", ids.len());
    assert!(ids.contains(&KNOWN_ID), "list should contain {KNOWN_ID}");
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "--list output must be sorted");
}

#[test]
fn explain_list_json_is_an_array_of_entries() {
    let out = explain(&["--list", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    let array = value.as_array().expect("list --json is an array");
    assert!(array.len() >= 200);
    let first = &array[0];
    assert!(first.get("rule_id").is_some());
    assert!(first.get("severity").is_some());
    assert!(first.get("summary").is_some());
}
