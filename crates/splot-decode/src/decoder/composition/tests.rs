// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use splot_core::headers::atlas_segment::AtlasMultistreamSegment;
use splot_core::simd::{SimdPolicy, resolve_backend};
use splot_recon::{AtlasSample, ReconSample};

fn view<T: ReconSample>(samples: &[T], w: usize, h: usize) -> PlaneRef<'_, T> {
    PlaneRef::new(samples, w, PlaneRect::new(0, 0, w, h).unwrap()).unwrap()
}

fn segment(id: u8, x: u32, y: u32, w: u32, h: u32) -> AtlasMultistreamSegment {
    AtlasMultistreamSegment {
        input_stream_id: id,
        top_left_pos_x: x,
        top_left_pos_y: y,
        width: w,
        height: h,
        alpha_segment_flag: false,
    }
}

struct CheckPlacement;
impl SimdScope<()> for CheckPlacement {
    fn run<B: SimdBackend>(self, backend: B) {
        let g = Geometry {
            size: [4, 2],
            chroma: [4, 2],
            depth: BitDepth::Eight,
            format: PixelFormat::Yuv444,
            samples: 24,
        };
        let mut work = Storage::default();
        let a = [10, 20, 30, 40];
        let b = [100, 110, 120, 130];
        for (id, delta) in [(PlaneId::Y, 0), (PlaneId::U, 3), (PlaneId::V, 7)] {
            let mut dest = [99u8; 8];
            for (s, data) in [(segment(9, 2, 0, 2, 2), b), (segment(4, 0, 0, 2, 2), a)] {
                let src = data.map(|n| n + delta);
                render::paint(
                    backend,
                    &mut dest,
                    &mut [],
                    &mut work,
                    g,
                    id,
                    &s,
                    view(&src, 2, 2),
                    None,
                )
                .unwrap();
            }
            assert_eq!(
                dest,
                [10, 20, 100, 110, 30, 40, 120, 130].map(|n| n + delta)
            );
        }
        let mut dest = [99u8; 8];
        for (s, data) in [(segment(4, 1, 0, 2, 2), a), (segment(9, 2, 1, 4, 2), b)] {
            render::paint(
                backend,
                &mut dest,
                &mut [],
                &mut work,
                g,
                PlaneId::Y,
                &s,
                view(&data, 2, 2),
                None,
            )
            .unwrap();
        }
        assert_eq!(dest, [99, 10, 20, 99, 99, 30, 100, 100]);
    }
}

#[test]
fn placement_preserves_planes_order_background_and_clips_after_scaling() {
    resolve_backend(SimdPolicy::Auto)
        .unwrap()
        .enter(CheckPlacement);
}

struct CheckAlpha;
impl SimdScope<()> for CheckAlpha {
    fn run<B: SimdBackend>(self, backend: B) {
        let g = Geometry {
            size: [8, 4],
            chroma: [4, 2],
            depth: BitDepth::Ten,
            format: PixelFormat::Yuv420,
            samples: 48,
        };
        let mut alpha = Storage::default();
        let mut temp = [0; 8];
        let mut dest = [20u16; 8];
        let weights = [0u8, 63, 128, 93, 255, 0, 0, 0, 0, 0];
        let mask = PictureSamples::Eight(view(&weights, 5, 2));
        let tex = [220u16; 8];
        render::paint(
            backend,
            &mut dest,
            &mut temp,
            &mut alpha,
            g,
            PlaneId::U,
            &segment(4, 0, 0, 8, 4),
            view(&tex, 4, 2),
            Some((&segment(9, 2, 0, 5, 2), mask)),
        )
        .unwrap();
        assert_eq!(dest, [220, 20, 120, 219, 220, 220, 220, 220]);
        let mut output = [0u16; 33];
        let source: Vec<_> = (0..701).map(|n| n as u16).collect();
        for width in [1, 7, 130, 389, 1024] {
            let len = (width as usize).min(output.len());
            u16::nearest_atlas_row(backend, &mut output[..len], &source, width, 0, false).unwrap();
            for (i, &sample) in output[..len].iter().enumerate() {
                assert_eq!(sample as usize, i * 701 / width as usize);
            }
        }
    }
}

#[test]
fn chroma_uses_offset_alpha_and_full_opacity_outside_its_rectangle() {
    resolve_backend(SimdPolicy::Auto).unwrap().enter(CheckAlpha);
}

#[test]
fn limited_range_background_uses_bt709_coefficients() {
    assert_eq!(render::background(None, BitDepth::Eight), [16, 128, 128]);
    assert_eq!(render::background(None, BitDepth::Ten), [64, 512, 512]);
    assert_eq!(
        render::background(Some((235, 16, 16)), BitDepth::Eight),
        [63, 102, 240]
    );
}

#[test]
fn invalid_pairing_format_and_geometry_fail_before_allocation() {
    let limits = DecodeLimits::default();
    let texture = segment(3, 0, 0, 4, 4);
    let alpha = AtlasMultistreamSegment {
        alpha_segment_flag: true,
        ..segment(9, 0, 0, 4, 4)
    };
    let second_alpha = AtlasMultistreamSegment {
        input_stream_id: 1,
        ..alpha
    };
    assert!(matches!(
        validate_segments(&[alpha], limits),
        Err(AtlasCompositionError::InvalidLayout)
    ));
    assert!(matches!(
        validate_segments(&[alpha, second_alpha, texture], limits),
        Err(AtlasCompositionError::InvalidLayout)
    ));
    assert!(matches!(
        validate_segments(&[texture, texture], limits),
        Err(AtlasCompositionError::InvalidLayout)
    ));
    assert!(matches!(
        Geometry::new(
            [u32::MAX; 2],
            PixelFormat::Yuv444,
            BitDepth::Ten,
            limits,
            true
        ),
        Err(AtlasCompositionError::InvalidLayout)
    ));
    let limited = limits.with_max_decoded_frame_bytes(DecodeLimitThreshold::Max(64));
    assert!(matches!(
        Geometry::new([4; 2], PixelFormat::Yuv444, BitDepth::Ten, limited, false),
        Err(AtlasCompositionError::Limit(_))
    ));
    let hdr = DerivedColorInfo {
        code_points: ColorPrimariesTriple {
            color_primaries: 9,
            transfer_characteristics: 16,
            matrix_coefficients: 9,
        },
        full_range: false,
    };
    assert!(!compatible_color(hdr));
}
