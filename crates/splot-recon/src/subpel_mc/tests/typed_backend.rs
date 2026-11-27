// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Differential tests comparing typed `SimdBackend` entrypoints against
//! compatibility dispatch routes.

use super::*;

#[test]
fn typed_backend_matches_compatibility_routes() {
    use splot_core::simd::{SimdBackend, SimdPolicy, SimdScope, resolve_backend};

    struct Check;
    impl SimdScope<()> for Check {
        fn run<B: SimdBackend>(self, backend: B) {
            let (ref_w, ref_h) = (48usize, 40usize);
            let ref0_8: Vec<u8> = (0..ref_w * ref_h)
                .map(|i| ((i * 37 + (i / ref_w) * 19 + 5) % 256) as u8)
                .collect();
            let ref1_8: Vec<u8> = (0..ref_w * ref_h)
                .map(|i| ((i * 53 + (i / ref_w) * 11 + 17) % 256) as u8)
                .collect();
            let ref0_10: Vec<u16> = (0..ref_w * ref_h)
                .map(|i| ((i * 101 + (i / ref_w) * 29 + 7) % 1024) as u16)
                .collect();
            let ref1_10: Vec<u16> = (0..ref_w * ref_h)
                .map(|i| ((i * 131 + (i / ref_w) * 41 + 13) % 1024) as u16)
                .collect();
            let view0_8 = ReferencePlaneView::new(&ref0_8, ref_w, ref_h).unwrap();
            let view1_8 = ReferencePlaneView::new(&ref1_8, ref_w, ref_h).unwrap();
            let view0_10 = ReferencePlaneView::new(&ref0_10, ref_w, ref_h).unwrap();
            let view1_10 = ReferencePlaneView::new(&ref1_10, ref_w, ref_h).unwrap();

            let p8_a = SubpelPredictParams {
                start_x: (6 << SCALE_SUBPEL_BITS) + (5 << 6),
                start_y: (4 << SCALE_SUBPEL_BITS) + (9 << 6),
                ..full_pel_params(
                    InterpolationFilter::EightTap,
                    16,
                    8,
                    6,
                    4,
                    ref_w as i32,
                    ref_h as i32,
                )
            };
            let p8_b = SubpelPredictParams {
                start_x: (-1 << SCALE_SUBPEL_BITS) + (3 << 6),
                start_y: (7 << SCALE_SUBPEL_BITS) + (11 << 6),
                first_x: 1,
                last_x: 20,
                ..full_pel_params(
                    InterpolationFilter::EightTapSmooth,
                    16,
                    8,
                    -1,
                    7,
                    ref_w as i32,
                    ref_h as i32,
                )
            };
            let p10_a = SubpelPredictParams {
                bit_depth: BitDepth::Ten,
                interp: InterpolationFilter::EightTapSharp,
                ..p8_a
            };
            let p10_b = SubpelPredictParams {
                bit_depth: BitDepth::Ten,
                ..p8_b
            };
            let p10_scaled = SubpelPredictParams {
                step_x: 1200,
                step_y: 900,
                ..p10_a
            };

            let stride = 20usize;
            for (v, p) in [(&view0_8, &p8_a), (&view0_8, &p8_b)] {
                let mut c_flat = vec![0u16; p.w * p.h];
                let mut t_flat = vec![0u16; p.w * p.h];
                subpel_predict_block_into(v, p, &mut c_flat).unwrap();
                subpel_predict_block_into_with(backend, v, p, &mut t_flat).unwrap();
                assert_eq!(t_flat, c_flat);

                let mut c_strided = vec![0x5a5a_u16; stride * p.h];
                let mut t_strided = vec![0x5a5a_u16; stride * p.h];
                subpel_predict_block_strided_into(v, p, &mut c_strided, stride).unwrap();
                subpel_predict_block_strided_into_with(backend, v, p, &mut t_strided, stride)
                    .unwrap();
                assert_eq!(t_strided, c_strided);

                let mut c_u8 = vec![0xa5_u8; stride * p.h];
                let mut t_u8 = vec![0xa5_u8; stride * p.h];
                subpel_predict_block_strided_into_u8(v, p, &mut c_u8, stride).unwrap();
                subpel_predict_block_strided_into_u8_with(backend, v, p, &mut t_u8, stride)
                    .unwrap();
                assert_eq!(t_u8, c_u8);
            }
            for (v, p) in [
                (&view0_10, &p10_a),
                (&view0_10, &p10_b),
                (&view0_10, &p10_scaled),
            ] {
                let mut c_flat = vec![0u16; p.w * p.h];
                let mut t_flat = vec![0u16; p.w * p.h];
                subpel_predict_block_into(v, p, &mut c_flat).unwrap();
                subpel_predict_block_into_with(backend, v, p, &mut t_flat).unwrap();
                assert_eq!(t_flat, c_flat);

                let mut c_strided = vec![0x5a5a_u16; stride * p.h];
                let mut t_strided = vec![0x5a5a_u16; stride * p.h];
                subpel_predict_block_strided_into(v, p, &mut c_strided, stride).unwrap();
                subpel_predict_block_strided_into_with(backend, v, p, &mut t_strided, stride)
                    .unwrap();
                assert_eq!(t_strided, c_strided);
            }

            let mut c_pred0_narrow = vec![0i16; stride * p8_a.h];
            let mut t_pred0_narrow = vec![0i16; stride * p8_a.h];
            subpel_predict_block_compound_intermediate_into(
                &view0_8,
                &p8_a,
                None,
                &mut c_pred0_narrow,
                stride,
            )
            .unwrap();
            subpel_predict_block_compound_intermediate_into_with(
                backend,
                &view0_8,
                &p8_a,
                None,
                &mut t_pred0_narrow,
                stride,
            )
            .unwrap();
            assert_eq!(t_pred0_narrow, c_pred0_narrow);

            let pred0_narrow_compact: Vec<i16> = (0..p8_a.h)
                .flat_map(|r| {
                    c_pred0_narrow[r * stride..r * stride + p8_a.w]
                        .iter()
                        .copied()
                })
                .collect();
            let pred0_wide_8: Vec<i32> =
                pred0_narrow_compact.iter().map(|&v| i32::from(v)).collect();

            let mut c_avg_u8 = vec![0u8; stride * p8_a.h];
            let mut t_avg_u8 = vec![0u8; stride * p8_a.h];
            subpel_predict_block_compound_average_strided_into_u8(
                &view1_8,
                &p8_b,
                &pred0_narrow_compact,
                6,
                None,
                &mut c_avg_u8,
                stride,
            )
            .unwrap();
            subpel_predict_block_compound_average_strided_into_u8_with(
                backend,
                &view1_8,
                &p8_b,
                &pred0_narrow_compact,
                6,
                None,
                &mut t_avg_u8,
                stride,
            )
            .unwrap();
            assert_eq!(t_avg_u8, c_avg_u8);

            let mut c_avg_u8_w = vec![0u8; stride * p8_a.h];
            let mut t_avg_u8_w = vec![0u8; stride * p8_a.h];
            subpel_predict_block_compound_average_strided_into_u8(
                &view1_8,
                &p8_a,
                &pred0_wide_8,
                11,
                None,
                &mut c_avg_u8_w,
                stride,
            )
            .unwrap();
            subpel_predict_block_compound_average_strided_into_u8_with(
                backend,
                &view1_8,
                &p8_a,
                &pred0_wide_8,
                11,
                None,
                &mut t_avg_u8_w,
                stride,
            )
            .unwrap();
            assert_eq!(t_avg_u8_w, c_avg_u8_w);

            let mut c_pred0_wide = vec![0i32; stride * p10_a.h];
            let mut t_pred0_wide = vec![0i32; stride * p10_a.h];
            subpel_predict_block_compound_intermediate_into(
                &view0_10,
                &p10_a,
                None,
                &mut c_pred0_wide,
                stride,
            )
            .unwrap();
            subpel_predict_block_compound_intermediate_into_with(
                backend,
                &view0_10,
                &p10_a,
                None,
                &mut t_pred0_wide,
                stride,
            )
            .unwrap();
            assert_eq!(t_pred0_wide, c_pred0_wide);

            let pred0_wide_10: Vec<i32> = (0..p10_a.h)
                .flat_map(|r| {
                    c_pred0_wide[r * stride..r * stride + p10_a.w]
                        .iter()
                        .copied()
                })
                .collect();
            let mut c_avg_u16 = vec![0u16; stride * p10_a.h];
            let mut t_avg_u16 = vec![0u16; stride * p10_a.h];
            subpel_predict_block_compound_average_strided_into(
                &view1_10,
                &p10_b,
                &pred0_wide_10,
                9,
                None,
                &mut c_avg_u16,
                stride,
            )
            .unwrap();
            subpel_predict_block_compound_average_strided_into_with(
                backend,
                &view1_10,
                &p10_b,
                &pred0_wide_10,
                9,
                None,
                &mut t_avg_u16,
                stride,
            )
            .unwrap();
            assert_eq!(t_avg_u16, c_avg_u16);

            let mut c_unscaled_u8 = vec![0u8; stride * p8_a.h];
            let mut t_unscaled_u8 = vec![0u8; stride * p8_a.h];
            assert_eq!(
                subpel_predict_block_compound_average_unscaled_strided_into(
                    &view0_8,
                    &p8_a,
                    &view1_8,
                    &p8_b,
                    8,
                    &mut c_unscaled_u8,
                    stride,
                ),
                Ok(true)
            );
            assert_eq!(
                subpel_predict_block_compound_average_unscaled_strided_into_with(
                    backend,
                    &view0_8,
                    &p8_a,
                    &view1_8,
                    &p8_b,
                    8,
                    &mut t_unscaled_u8,
                    stride,
                ),
                Ok(true)
            );
            assert_eq!(t_unscaled_u8, c_unscaled_u8);

            let mut c_fast_u16 = vec![0u16; stride * p10_a.h];
            let mut t_fast_u16 = vec![0u16; stride * p10_a.h];
            assert_eq!(
                subpel_predict_block_compound_average_fast_validated_strided_into(
                    &view0_10,
                    &p10_a,
                    &view1_10,
                    &p10_b,
                    10,
                    &mut c_fast_u16,
                    stride,
                ),
                Ok(true)
            );
            assert_eq!(
                subpel_predict_block_compound_average_fast_validated_strided_into_with(
                    backend,
                    &view0_10,
                    &p10_a,
                    &view1_10,
                    &p10_b,
                    10,
                    &mut t_fast_u16,
                    stride,
                ),
                Ok(true)
            );
            assert_eq!(t_fast_u16, c_fast_u16);

            let info_8 = crate::DecodedFrameInfo::new(
                crate::OutputIndex::new(0),
                BitDepth::Eight,
                crate::PixelFormat::Yuv420,
                crate::PlaneSize::new(ref_w, ref_h).unwrap(),
                crate::PlaneRect::new(0, 0, ref_w, ref_h).unwrap(),
            )
            .unwrap();
            let (uv_w, uv_h) = (ref_w / 2, ref_h / 2);
            let uv0_8 = ReferencePlaneView::new(&ref0_8[..uv_w * uv_h], uv_w, uv_h).unwrap();
            let uv1_8 = ReferencePlaneView::new(&ref1_8[..uv_w * uv_h], uv_w, uv_h).unwrap();
            let luma_put0 = UnscaledPut {
                rect: crate::PlaneRect::new(8, 8, 16, 8).unwrap(),
                start: [p8_a.start_x, p8_a.start_y],
                last: [ref_w as i32 - 1, ref_h as i32 - 1],
            };
            let luma_put1 = UnscaledPut {
                rect: crate::PlaneRect::new(8, 8, 16, 8).unwrap(),
                start: [p8_b.start_x, p8_b.start_y],
                last: [ref_w as i32 - 1, ref_h as i32 - 1],
            };
            let chroma_put0 = UnscaledPut {
                rect: crate::PlaneRect::new(4, 4, 8, 4).unwrap(),
                start: [(3 << 10) + (4 << 6), (2 << 10) + (7 << 6)],
                last: [uv_w as i32 - 1, uv_h as i32 - 1],
            };
            let chroma_put1 = UnscaledPut {
                rect: crate::PlaneRect::new(4, 4, 8, 4).unwrap(),
                start: [(5 << 10) + (2 << 6), (4 << 10) + (12 << 6)],
                last: [uv_w as i32 - 1, uv_h as i32 - 1],
            };
            for compound in [false, true] {
                let mut c_ws = crate::CurrentFrameWorkspace::<u8>::new(info_8, 0).unwrap();
                let mut t_ws = crate::CurrentFrameWorkspace::<u8>::new(info_8, 0).unwrap();
                let (c_ok, t_ok) = if compound {
                    let mut c_surf = crate::CurrentFrameSurface::Frame(&mut c_ws);
                    let mut t_surf = crate::CurrentFrameSurface::Frame(&mut t_ws);
                    (
                        subpel_compound_block(
                            &mut c_surf,
                            InterpolationFilter::EightTapSharp,
                            7,
                            ([&view0_8, &view1_8], [luma_put0, luma_put1]),
                            Some((
                                [[&uv0_8, &uv1_8], [&uv1_8, &uv0_8]],
                                [chroma_put0, chroma_put1],
                            )),
                        ),
                        subpel_compound_block_with(
                            backend,
                            &mut t_surf,
                            InterpolationFilter::EightTapSharp,
                            7,
                            ([&view0_8, &view1_8], [luma_put0, luma_put1]),
                            Some((
                                [[&uv0_8, &uv1_8], [&uv1_8, &uv0_8]],
                                [chroma_put0, chroma_put1],
                            )),
                        ),
                    )
                } else {
                    let mut c_surf = crate::CurrentFrameSurface::Frame(&mut c_ws);
                    let mut t_surf = crate::CurrentFrameSurface::Frame(&mut t_ws);
                    (
                        subpel_put_block(
                            &mut c_surf,
                            InterpolationFilter::EightTapSharp,
                            (&view0_8, luma_put0),
                            Some(([&uv0_8, &uv1_8], chroma_put0)),
                        ),
                        subpel_put_block_with(
                            backend,
                            &mut t_surf,
                            InterpolationFilter::EightTapSharp,
                            (&view0_8, luma_put0),
                            Some(([&uv0_8, &uv1_8], chroma_put0)),
                        ),
                    )
                };
                assert_eq!(t_ok, [true, true]);
                assert_eq!(t_ok, c_ok);
                for (plane_id, rect) in [
                    (crate::PlaneId::Y, luma_put0.rect),
                    (crate::PlaneId::U, chroma_put0.rect),
                    (crate::PlaneId::V, chroma_put0.rect),
                ] {
                    let c_plane = c_ws.plane(plane_id).unwrap();
                    let t_plane = t_ws.plane(plane_id).unwrap();
                    for r in 0..rect.height() {
                        for c in 0..rect.width() {
                            assert_eq!(
                                t_plane.reconstructed_sample(rect.x() + c, rect.y() + r),
                                c_plane.reconstructed_sample(rect.x() + c, rect.y() + r)
                            );
                        }
                    }
                }
            }

            let info_10 = crate::DecodedFrameInfo::new(
                crate::OutputIndex::new(0),
                BitDepth::Ten,
                crate::PixelFormat::Yuv420,
                crate::PlaneSize::new(ref_w, ref_h).unwrap(),
                crate::PlaneRect::new(0, 0, ref_w, ref_h).unwrap(),
            )
            .unwrap();
            let uv0_10 = ReferencePlaneView::new(&ref0_10[..uv_w * uv_h], uv_w, uv_h).unwrap();
            let uv1_10 = ReferencePlaneView::new(&ref1_10[..uv_w * uv_h], uv_w, uv_h).unwrap();
            for compound in [false, true] {
                let mut c_ws = crate::CurrentFrameWorkspace::<u16>::new(info_10, 0).unwrap();
                let mut t_ws = crate::CurrentFrameWorkspace::<u16>::new(info_10, 0).unwrap();
                let (c_ok, t_ok) = if compound {
                    let mut c_surf = crate::CurrentFrameSurface::Frame(&mut c_ws);
                    let mut t_surf = crate::CurrentFrameSurface::Frame(&mut t_ws);
                    (
                        subpel_compound_block(
                            &mut c_surf,
                            InterpolationFilter::EightTapSharp,
                            7,
                            ([&view0_10, &view1_10], [luma_put0, luma_put1]),
                            Some((
                                [[&uv0_10, &uv1_10], [&uv1_10, &uv0_10]],
                                [chroma_put0, chroma_put1],
                            )),
                        ),
                        subpel_compound_block_with(
                            backend,
                            &mut t_surf,
                            InterpolationFilter::EightTapSharp,
                            7,
                            ([&view0_10, &view1_10], [luma_put0, luma_put1]),
                            Some((
                                [[&uv0_10, &uv1_10], [&uv1_10, &uv0_10]],
                                [chroma_put0, chroma_put1],
                            )),
                        ),
                    )
                } else {
                    let mut c_surf = crate::CurrentFrameSurface::Frame(&mut c_ws);
                    let mut t_surf = crate::CurrentFrameSurface::Frame(&mut t_ws);
                    (
                        subpel_put_block(
                            &mut c_surf,
                            InterpolationFilter::EightTapSharp,
                            (&view0_10, luma_put0),
                            Some(([&uv0_10, &uv1_10], chroma_put0)),
                        ),
                        subpel_put_block_with(
                            backend,
                            &mut t_surf,
                            InterpolationFilter::EightTapSharp,
                            (&view0_10, luma_put0),
                            Some(([&uv0_10, &uv1_10], chroma_put0)),
                        ),
                    )
                };
                assert_eq!(t_ok, [true, true]);
                assert_eq!(t_ok, c_ok);
                for (plane_id, rect) in [
                    (crate::PlaneId::Y, luma_put0.rect),
                    (crate::PlaneId::U, chroma_put0.rect),
                    (crate::PlaneId::V, chroma_put0.rect),
                ] {
                    let c_plane = c_ws.plane(plane_id).unwrap();
                    let t_plane = t_ws.plane(plane_id).unwrap();
                    for r in 0..rect.height() {
                        for c in 0..rect.width() {
                            assert_eq!(
                                t_plane.reconstructed_sample(rect.x() + c, rect.y() + r),
                                c_plane.reconstructed_sample(rect.x() + c, rect.y() + r)
                            );
                        }
                    }
                }
            }

            let mut cells = CompoundCells::default();
            let c0: CellPlacement = (
                [(6 << 10) + (5 << 6), (4 << 10) + (9 << 6)],
                [0, 0, ref_w as i32 - 1, ref_h as i32 - 1],
            );
            let c1: CellPlacement = (
                [7 << 10, 8 << 10],
                [0, 0, ref_w as i32 - 1, ref_h as i32 - 1],
            );
            assert!(cells.push([0, 0], [c0, c1]));
            assert!(cells.push([0, 8], [c1, c1]));
            let mut c_cells_u8 = vec![0u8; 8 * stride];
            let mut t_cells_u8 = vec![0u8; 8 * stride];
            let c_written_8 = subpel_compound_cells(
                [Some((&[view0_8, view1_8], &mut c_cells_u8, stride)), None],
                InterpolationFilter::EightTap,
                8,
                &mut cells.clone(),
                (11, BitDepth::Eight),
            );
            let t_written_8 = subpel_compound_cells_with(
                backend,
                [Some((&[view0_8, view1_8], &mut t_cells_u8, stride)), None],
                InterpolationFilter::EightTap,
                8,
                &mut cells.clone(),
                (11, BitDepth::Eight),
            );
            assert_eq!(t_written_8, c_written_8);
            assert_eq!(t_cells_u8, c_cells_u8);

            let mut c_cells_u16 = vec![0u16; 8 * stride];
            let mut t_cells_u16 = vec![0u16; 8 * stride];
            let c_written_10 = subpel_compound_cells(
                [
                    Some((&[view0_10, view1_10], &mut c_cells_u16, stride)),
                    None,
                ],
                InterpolationFilter::EightTap,
                8,
                &mut cells.clone(),
                (11, BitDepth::Ten),
            );
            let t_written_10 = subpel_compound_cells_with(
                backend,
                [
                    Some((&[view0_10, view1_10], &mut t_cells_u16, stride)),
                    None,
                ],
                InterpolationFilter::EightTap,
                8,
                &mut cells,
                (11, BitDepth::Ten),
            );
            assert_eq!(t_written_10, c_written_10);
            assert_eq!(t_cells_u16, c_cells_u16);

            let mut c_carry8 = BilinearOverlapCarry::default();
            let mut t_carry8 = BilinearOverlapCarry::default();
            let mut c_carry10 = BilinearOverlapCarry::default();
            let mut t_carry10 = BilinearOverlapCarry::default();
            for x in [6, 14] {
                let p_ov8 = SubpelPredictParams {
                    start_x: (x << SCALE_SUBPEL_BITS) + (7 << 6),
                    start_y: (4 << SCALE_SUBPEL_BITS) + (11 << 6),
                    ..full_pel_params(
                        InterpolationFilter::Bilinear,
                        12,
                        12,
                        x,
                        4,
                        ref_w as i32,
                        ref_h as i32,
                    )
                };
                let mut c_ov8 = vec![0u16; 16 * 12];
                let mut t_ov8 = vec![0u16; 16 * 12];
                assert_eq!(
                    subpel_predict_12x12_bilinear_overlap_into(
                        &view0_8,
                        &p_ov8,
                        &mut c_ov8,
                        16,
                        &mut c_carry8,
                    ),
                    Ok(true)
                );
                assert_eq!(
                    subpel_predict_12x12_bilinear_overlap_into_with(
                        backend,
                        &view0_8,
                        &p_ov8,
                        &mut t_ov8,
                        16,
                        &mut t_carry8,
                    ),
                    Ok(true)
                );
                assert_eq!(t_ov8, c_ov8);

                let p_ov10 = SubpelPredictParams {
                    bit_depth: BitDepth::Ten,
                    ..p_ov8
                };
                let mut c_ov10 = vec![0u16; 16 * 12];
                let mut t_ov10 = vec![0u16; 16 * 12];
                assert_eq!(
                    subpel_predict_12x12_bilinear_overlap_into(
                        &view0_10,
                        &p_ov10,
                        &mut c_ov10,
                        16,
                        &mut c_carry10,
                    ),
                    Ok(true)
                );
                assert_eq!(
                    subpel_predict_12x12_bilinear_overlap_into_with(
                        backend,
                        &view0_10,
                        &p_ov10,
                        &mut t_ov10,
                        16,
                        &mut t_carry10,
                    ),
                    Ok(true)
                );
                assert_eq!(t_ov10, c_ov10);
            }

            let p_bi = SubpelPredictParams {
                interp: InterpolationFilter::Bilinear,
                ..p8_a
            };
            let mut c_bi = vec![0u16; stride * p_bi.h];
            let mut t_bi = vec![0u16; stride * p_bi.h];
            assert!(subpel_predict_bilinear_u8(
                &view0_8, &p_bi, &mut c_bi, stride
            ));
            assert!(subpel_predict_bilinear_u8_with(
                backend, &view0_8, &p_bi, &mut t_bi, stride,
            ));
            assert_eq!(t_bi, c_bi);

            assert_eq!(
                refinemv_area_sad_u8_with(backend, [&view0_8, &view1_8], [[4, 4], [6, 8]], [12, 8]),
                refinemv_area_sad_u8([&view0_8, &view1_8], [[4, 4], [6, 8]], [12, 8])
            );

            let rmv_unclamped = [0, 1].map(|idx| SubpelPredictParams {
                interp: InterpolationFilter::Bilinear,
                w: 16,
                h: 16,
                start_x: ((6 + idx) << SCALE_SUBPEL_BITS) + (3 << 6),
                start_y: ((5 + idx) << SCALE_SUBPEL_BITS) + (7 << 6),
                first_x: 7 + idx,
                last_x: 21 + idx,
                first_y: 6 + idx,
                last_y: 20 + idx,
                ..full_pel_params(
                    InterpolationFilter::Bilinear,
                    16,
                    16,
                    6 + idx,
                    5 + idx,
                    ref_w as i32,
                    ref_h as i32,
                )
            });
            let rmv_clamped = [0, 1].map(|idx| SubpelPredictParams {
                first_x: 0,
                first_y: 0,
                last_x: ref_w as i32 - 1,
                last_y: ref_h as i32 - 1,
                ..rmv_unclamped[idx]
            });
            for (rmv, allow_center) in [
                (&rmv_unclamped, false),
                (&rmv_unclamped, true),
                (&rmv_clamped, true),
            ] {
                let mut c_w8 = [[0u8; REFINEMV_SEARCH_AREA]; 2];
                let mut t_w8 = [[0u8; REFINEMV_SEARCH_AREA]; 2];
                assert_eq!(
                    refinemv_search_windows_with(
                        backend,
                        [&view0_8, &view1_8],
                        [&rmv[0], &rmv[1]],
                        allow_center,
                        BitDepth::Eight,
                        t_w8.each_mut(),
                    ),
                    refinemv_search_windows(
                        [&view0_8, &view1_8],
                        [&rmv[0], &rmv[1]],
                        allow_center,
                        BitDepth::Eight,
                        c_w8.each_mut(),
                    )
                );
                assert_eq!(t_w8, c_w8);

                let rmv_10 = [0, 1].map(|i| SubpelPredictParams {
                    bit_depth: BitDepth::Ten,
                    ..rmv[i]
                });
                let mut c_w16 = [[0u16; REFINEMV_SEARCH_AREA]; 2];
                let mut t_w16 = [[0u16; REFINEMV_SEARCH_AREA]; 2];
                assert_eq!(
                    refinemv_search_windows_with(
                        backend,
                        [&view0_10, &view1_10],
                        [&rmv_10[0], &rmv_10[1]],
                        allow_center,
                        BitDepth::Ten,
                        t_w16.each_mut(),
                    ),
                    refinemv_search_windows(
                        [&view0_10, &view1_10],
                        [&rmv_10[0], &rmv_10[1]],
                        allow_center,
                        BitDepth::Ten,
                        c_w16.each_mut(),
                    )
                );
                assert_eq!(t_w16, c_w16);
            }
        }
    }

    resolve_backend(SimdPolicy::Auto)
        .expect("auto backend")
        .enter(Check);
}
