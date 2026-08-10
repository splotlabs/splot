// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#[test]
fn missing_capability_message_formats_id_and_context() {
    assert_eq!(
        super::missing_capability_message!("intra.10bit.non_dc"),
        "unsupported capability: intra.10bit.non_dc"
    );
    assert_eq!(
        super::missing_capability_message!(
            "intra.directional.d45",
            plane = "luma",
            neighbour = "above_right",
            block = "64x64",
        ),
        "unsupported capability: intra.directional.d45 plane=luma neighbour=above_right block=64x64"
    );
}
