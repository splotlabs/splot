// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::{assert_yuv420_8bit_frames, decode_fixture, frame_hashes};

const SEGMENTATION_INHERIT_FIXTURE: &[u8] = include_bytes!(
    "../../../../../../tests/conformance/vectors/valid/syn-3frame-seg-inherit-inter-64x64.ivf"
);

const SEGMENTATION_SKIP_FIXTURE: &[u8] = include_bytes!(
    "../../../../../../tests/conformance/vectors/valid/syn-2frame-seg-skip-inter-64x64.ivf"
);

#[test]
fn empty_segment_map_uses_spec_mi_dimensions_for_odd_frames() {
    let (_, mut core, _) = super::parse_inter_core_for_validation(SEGMENTATION_SKIP_FIXTURE)
        .expect("fixture has a valid inter frame header");
    let size = core.frame_size.as_mut().expect("fixture has a frame size");
    size.width = 9;
    size.height = 9;
    let tile_info = core.tile_info.as_mut().expect("fixture has tile info");
    *tile_info
        .mi_col_starts
        .last_mut()
        .expect("tile columns include the frame edge") = 4;
    *tile_info
        .mi_row_starts
        .last_mut()
        .expect("tile rows include the frame edge") = 4;

    let map = super::super::empty_segment_id_map(&core).expect("MI dimensions are valid");
    assert_eq!(map.dimensions(), (4, 4));
}

#[test]
fn skip_segment_forces_skip_and_globalmv_decode_paths() {
    let (_, core, _) = super::parse_inter_core_for_validation(SEGMENTATION_SKIP_FIXTURE)
        .expect("fixture has a valid inter frame header");
    let segmentation = core
        .segmentation_params
        .expect("fixture enables segmentation");
    let seg_lvl_skip = 1;
    assert!(segmentation.segmentation_enabled);
    assert!(segmentation.segmentation_update_map);
    assert!(segmentation.seg_id_pre_skip);
    assert!(segmentation.features[7][seg_lvl_skip].enabled);

    let frames = decode_fixture(SEGMENTATION_SKIP_FIXTURE);
    assert_eq!(frames.len(), 2, "key frame + SKIP-segment inter frame");
    assert_yuv420_8bit_frames(&frames, 64, 64);
    assert_eq!(
        frame_hashes(&frames),
        [
            "01b3da14663aeb93e50236b18d719224e30be3c8feaa71a67a88bb1cf6946bd6",
            "5338821039316d18b02199df911cc42f56b5a687c5cb27649982bc236a4ba097",
        ],
        "frame hashes pinned from avmdec's byte-identical raw output"
    );
}

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
