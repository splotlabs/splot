// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Golden snapshot tests of the `splot inspect --json` output (CONF-INSPECT-SNAPSHOTS).
//!
//! The inspector's per-OBU JSON summary is fully deterministic for a fixed input (byte
//! offsets, sizes, parsed fields — no paths, timestamps, or filenames), so an `insta`
//! golden snapshot freezes the inspector's behavior: any future change to the inspect
//! output for a committed fixture is surfaced as a snapshot diff for explicit review
//! (`cargo insta review`). The snapshots cover a diverse set of OBU types across the
//! committed `tests/fixtures/` corpus.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;

/// Runs `splot inspect --json <fixture>` against a committed fixture and returns its stdout.
fn inspect_json(fixture: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(fixture);
    let out = Command::new(env!("CARGO_BIN_EXE_splot"))
        .args([
            "inspect",
            "--json",
            path.to_str().expect("fixture path is UTF-8"),
        ])
        .output()
        .expect("failed to run the splot binary");
    assert!(
        out.status.success(),
        "`splot inspect --json {fixture}` exited with {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8(out.stdout).expect("inspect stdout is valid UTF-8")
}

/// Defines one named golden snapshot test over a committed fixture.
macro_rules! inspect_snapshot {
    ($test:ident, $fixture:literal) => {
        #[test]
        fn $test() {
            insta::assert_snapshot!($fixture, inspect_json($fixture));
        }
    };
}

inspect_snapshot!(inspect_conformant, "conformant.av2");
inspect_snapshot!(inspect_operating_point_set, "operating-point-set.av2");
inspect_snapshot!(inspect_film_grain, "film-grain.av2");
inspect_snapshot!(inspect_buffer_removal_timing, "buffer-removal-timing.av2");
inspect_snapshot!(inspect_metadata_group, "metadata-group.av2");
inspect_snapshot!(inspect_metadata_short, "metadata-short.av2");
inspect_snapshot!(inspect_frame_header_core, "frame-header-core.av2");
inspect_snapshot!(inspect_frame_header_core_mfh, "frame-header-core-mfh.av2");
inspect_snapshot!(inspect_frame_header_prefix, "frame-header-prefix.av2");
