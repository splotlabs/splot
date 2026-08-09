// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::{assert_yuv420_8bit_frames, decode_fixture, frame_hashes};

const SEGMENTATION_INHERIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../../../tests/conformance/vectors/valid/syn-3frame-seg-inherit-inter-64x64.ivf"
);

#[test]
fn inherited_inter_segmentation_map_decodes_expected_frames() {
    let frames = decode_fixture(SEGMENTATION_INHERIT_FIXTURE);
    assert_eq!(frames.len(), 3, "key frame + two inter frames");
    assert_yuv420_8bit_frames(&frames, 64, 64);
    assert_eq!(
        frame_hashes(&frames),
        [
            "90979d1215aae92aa9d992589e1df94d80b2bd54fe254eda08207bcd40439dad",
            "89b74d605b68cb78ec568afa333e1e9aee588a3106601286a0d52984d7eedf91",
            "124b590c9753db2d9b53439005d00ec009006bd68d6632138da784ab1764c12b",
        ],
        "frame hashes pinned from avmdec's byte-identical raw output"
    );
    assert_eq!(
        frames[1].segment_ids.product(),
        frames[2].segment_ids.product(),
        "segmentation_update_map == 0 preserves the previous MI map"
    );
}
