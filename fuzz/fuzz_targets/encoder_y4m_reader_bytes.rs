// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_encode::y4m::Y4mReader;

const MAX_FRAMES: usize = 4;

fuzz_target!(|data: &[u8]| {
    let Ok(mut reader) = Y4mReader::new(data) else {
        return;
    };
    let header = *reader.header();
    assert!(header.width >= 1 && header.height >= 1);
    assert!((8..=16).contains(&header.bit_depth));
    let sizes = header.plane_sizes();
    for _ in 0..MAX_FRAMES {
        let Ok(Some(planes)) = reader.read_frame() else {
            return;
        };
        for (plane, (width, height)) in planes.iter().zip(sizes) {
            assert_eq!(plane.len(), width * height);
        }
    }
});
