// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Golden snapshot tests of the `splot inspect` HUMAN (text) output
//! (CONF-CLI-SNAPSHOT-COVERAGE).
//!
//! [`inspect_snapshots`](super) freezes the `--json` surface; this suite freezes
//! the complementary text surfaces — the default per-OBU dump and the `--headers`
//! header-only dump — which had no snapshot coverage. The text output is
//! deterministic for a fixed input (OBU index, byte offset, size, type, layer ids,
//! and the payload-length line in the default mode; no paths, timestamps, or
//! filenames), so any future change to the human dump for a committed fixture
//! surfaces as a reviewable snapshot diff (`cargo insta review`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;

/// Runs `splot inspect [mode] <fixture>` against a committed fixture and returns
/// its stdout, asserting a clean exit.
fn inspect_text(fixture: &str, mode: &[&str]) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(fixture);
    let mut args = vec!["inspect"];
    args.extend_from_slice(mode);
    args.push(path.to_str().expect("fixture path is UTF-8"));
    let out = Command::new(env!("CARGO_BIN_EXE_splot"))
        .args(&args)
        .output()
        .expect("failed to run the splot binary");
    assert!(
        out.status.success(),
        "`splot inspect {mode:?} {fixture}` exited with {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8(out.stdout).expect("inspect stdout is valid UTF-8")
}

/// Defines one named golden text snapshot over a committed fixture in a given mode.
macro_rules! inspect_text_snapshot {
    ($test:ident, $name:literal, $fixture:literal, $mode:expr) => {
        #[test]
        fn $test() {
            insta::assert_snapshot!($name, inspect_text($fixture, $mode));
        }
    };
}

// Header-only dump (`--headers`): omits the per-OBU payload-length line.
inspect_text_snapshot!(
    headers_conformant,
    "headers_conformant",
    "conformant.av2",
    &["--headers"]
);
inspect_text_snapshot!(
    headers_operating_point_set,
    "headers_operating_point_set",
    "operating-point-set.av2",
    &["--headers"]
);
inspect_text_snapshot!(
    headers_frame_header_core,
    "headers_frame_header_core",
    "frame-header-core.av2",
    &["--headers"]
);
inspect_text_snapshot!(
    headers_metadata_group,
    "headers_metadata_group",
    "metadata-group.av2",
    &["--headers"]
);

// Default dump: includes the per-OBU `payload: N byte(s)` line.
inspect_text_snapshot!(
    default_conformant,
    "default_conformant",
    "conformant.av2",
    &[]
);
inspect_text_snapshot!(
    default_frame_header_core,
    "default_frame_header_core",
    "frame-header-core.av2",
    &[]
);
