// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Golden JSON snapshots for committed OBU fixtures (CONF-INSPECT-SNAPSHOTS).

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
inspect_snapshot!(inspect_quantizer_matrix, "quantizer-matrix.av2");
inspect_snapshot!(inspect_seq_header_tile_params, "seq-header-tile-params.av2");
inspect_snapshot!(inspect_padding, "padding.av2");
