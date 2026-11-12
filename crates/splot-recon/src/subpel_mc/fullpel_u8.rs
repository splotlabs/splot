// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// Writes one eight-bit single-reference prediction into row-strided `u8` storage.
///
/// `output` starts at the prediction's top-left sample. The complete parameter
/// and output geometry is validated before the first sample is written.
///
/// # Errors
/// Returns the same parameter and output-layout errors as
/// [`subpel_predict_block_strided_into`], and rejects non-eight-bit output or
/// inverted reference clipping bounds before writing any sample.
pub fn subpel_predict_block_strided_into_u8<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u8],
    output_stride: usize,
) -> Result<()> {
    let (w, h) = (params.w, params.h);
    if let Some(view) = reference.as_u8()
        && params.bit_depth == BitDepth::Eight
        && params.step_x == 1 << SCALE_SUBPEL_BITS
        && params.step_y == 1 << SCALE_SUBPEL_BITS
        && (1..=MAX_BLOCK_DIM).contains(&w)
        && (1..=MAX_BLOCK_DIM).contains(&h)
        && params.first_x <= params.last_x
        && params.first_y <= params.last_y
        && params
            .start_x
            .checked_add((w as i32 - 1) << SCALE_SUBPEL_BITS)
            .is_some()
        && output_stride >= w
        && (h - 1)
            .checked_mul(output_stride)
            .and_then(|end| end.checked_add(w))
            .is_some_and(|end| end <= output.len())
    {
        let window = block::Window::new(params.interp, w, h, [params.start_x, params.start_y]);
        if block::with_block(&window, &view, params.bounds(), 0, |block| {
            super::eight_bit::put(block, output, output_stride);
        })
        .is_some()
        {
            return Ok(());
        }
    }
    validate_subpel_u8_output(params, output, output_stride)?;
    let sink = Sink::Put {
        output,
        stride: output_stride,
    };
    block::predict(reference, params, sink, None);
    Ok(())
}

/// One plane of an unscaled block from one reference: its output rectangle
/// and the § 7.13.3.18 1/1024-sample start `[startX, startY]` of its window,
/// whose reads clip to reference columns `0..=last[0]` and rows
/// `0..=last[1]`.
#[derive(Clone, Copy, Debug)]
pub struct UnscaledPut {
    /// The output rectangle.
    pub rect: crate::PlaneRect,
    /// `[startX, startY]`.
    pub start: [i32; 2],
    /// `[lastX, lastY]`.
    pub last: [i32; 2],
}

impl UnscaledPut {
    /// The window of this plane and its clip bounds, or `None` for a side
    /// above 256 samples, a negative last sample or a start whose last
    /// column overflows.
    #[inline]
    fn window(&self, interp: InterpolationFilter) -> Option<(block::Window, [i32; 4])> {
        let (w, h) = (self.rect.width(), self.rect.height());
        let [last_x, last_y] = self.last;
        (w <= MAX_BLOCK_DIM
            && h <= MAX_BLOCK_DIM
            && last_x >= 0
            && last_y >= 0
            && self.start[0]
                .checked_add((w as i32 - 1) << SCALE_SUBPEL_BITS)
                .is_some())
        .then(|| {
            let window = block::Window::new(interp, w, h, self.start);
            (window, [0, 0, last_x, last_y])
        })
    }
}

/// Stores the single-reference prediction, clipped to the frame bit depth,
/// of a block's luma plane straight into `surface`, and with `chroma` that
/// of both chroma planes, which share one window, as
/// [`subpel_predict_block_strided_into`] stores each.
/// Returns, for luma and for chroma, whether it was stored; a part that was
/// not is left unwritten or partly written for the caller to predict again.
#[inline]
pub fn subpel_put_block<T: ReconSample>(
    surface: &mut crate::CurrentFrameSurface<'_, '_, T>,
    interp: InterpolationFilter,
    (view, luma): (&ReferencePlaneView<'_, T>, UnscaledPut),
    chroma: Option<([&ReferencePlaneView<'_, T>; 2], UnscaledPut)>,
) -> [bool; 2] {
    let max = surface.info().bit_depth().max_sample();
    let luma_put = luma.window(interp).is_some_and(|(window, bounds)| {
        surface
            .rect_mut(crate::PlaneId::Y, luma.rect)
            .is_some_and(|(output, stride)| put_window(&window, bounds, view, max, output, stride))
    });
    let chroma_put = chroma.is_some_and(|([u, v], put)| {
        let Some((window, bounds)) = put.window(interp) else {
            return false;
        };
        surface
            .chroma_rects_mut(put.rect)
            .is_some_and(|[u_out, v_out]| put_pair(&window, bounds, [u, v], max, [u_out, v_out]))
    });
    [luma_put, chroma_put]
}

/// Both references' views of one plane.
type Views<'a, 'b, T> = [&'a ReferencePlaneView<'b, T>; 2];

/// The COMPOUND_AVERAGE of an unscaled block from two references, as
/// [`subpel_predict_block_compound_average_fast_validated_strided_into`]
/// stores each plane: luma, and with `chroma` both chroma planes, the U
/// views first, which share their windows. Both references of a part must
/// share its rectangle. Returns whether each part was stored, as
/// [`subpel_put_block`] does.
#[inline]
pub fn subpel_compound_block<T: ReconSample>(
    surface: &mut crate::CurrentFrameSurface<'_, '_, T>,
    interp: InterpolationFilter,
    cwp_weight: i16,
    (views, luma): (Views<'_, '_, T>, [UnscaledPut; 2]),
    chroma: Option<([Views<'_, '_, T>; 2], [UnscaledPut; 2])>,
) -> [bool; 2] {
    let clip = (
        i32::from(cwp_weight),
        surface.info().bit_depth().max_sample(),
    );
    let windows = |[first, second]: [UnscaledPut; 2]| {
        let ((a, a_bounds), (b, b_bounds)) = (first.window(interp)?, second.window(interp)?);
        (first.rect == second.rect).then_some(([a, b], [a_bounds, b_bounds]))
    };
    let luma_put = windows(luma).is_some_and(|(windows, bounds)| {
        surface
            .rect_mut(crate::PlaneId::Y, luma[0].rect)
            .is_some_and(|(output, stride)| {
                compound_windows(views, windows, bounds, clip, output, stride)
            })
    });
    let chroma_put = chroma.is_some_and(|([u, v], puts)| {
        let Some((windows, bounds)) = windows(puts) else {
            return false;
        };
        surface.chroma_rects_mut(puts[0].rect).is_some_and(
            |[(u_out, u_stride), (v_out, v_stride)]| {
                compound_windows(u, windows, bounds, clip, u_out, u_stride)
                    && compound_windows(v, windows, bounds, clip, v_out, v_stride)
            },
        )
    });
    [luma_put, chroma_put]
}

/// [`put_window`] of two views of one geometry, U and V, into their
/// outputs: eight-bit planes read in place in one pair call.
#[allow(clippy::inline_always, reason = "measured put hot path")]
#[inline(always)]
fn put_pair<T: ReconSample>(
    window: &block::Window,
    bounds: [i32; 4],
    views: [&ReferencePlaneView<'_, T>; 2],
    max: u16,
    [(first, first_stride), (second, second_stride)]: [(&mut [T], usize); 2],
) -> bool {
    if let ([Some(u), Some(v)], Some(first), Some(second)) = (
        views.map(ReferencePlaneView::as_u8),
        T::u8_slice_mut(&mut *first),
        T::u8_slice_mut(&mut *second),
    ) && let (Some(u_origin), Some(v_origin)) = (
        window.origin(u.geometry(), bounds),
        window.origin(v.geometry(), bounds),
    ) {
        let blocks = [
            window.block(u.samples, u.stride, u_origin),
            window.block(v.samples, v.stride, v_origin),
        ];
        super::eight_bit::put_pair(
            [&blocks[0], &blocks[1]],
            [(first, first_stride), (second, second_stride)],
        );
        return true;
    }
    put_window(window, bounds, views[0], max, first, first_stride)
        && put_window(window, bounds, views[1], max, second, second_stride)
}

/// The `Clip1` single-reference prediction of `window` read from `view`
/// within `bounds` into `output`; `false` without writing for an empty clip
/// range or storage that is neither 8- nor 16-bit.
#[allow(clippy::inline_always, reason = "measured put hot path")]
#[inline(always)]
fn put_window<T: ReconSample>(
    window: &block::Window,
    bounds: [i32; 4],
    view: &ReferencePlaneView<'_, T>,
    max: u16,
    output: &mut [T],
    stride: usize,
) -> bool {
    if let (Some(view), Some(output)) = (view.as_u8(), T::u8_slice_mut(&mut *output)) {
        return block::with_block(window, &view, bounds, 0, |block| {
            super::eight_bit::put(block, output, stride);
        })
        .is_some();
    }
    let (Some(view), Some(output)) = (view.as_u16(), T::u16_slice_mut(output)) else {
        return false;
    };
    block::with_block(window, &view, bounds, 0, |block| {
        block::kernels(
            block,
            &mut Sink::PutWide {
                output,
                stride,
                max,
            },
            None,
        );
    })
    .is_some()
}

fn validate_subpel_u8_output(
    params: &SubpelPredictParams,
    output: &[u8],
    output_stride: usize,
) -> Result<usize> {
    crate::intra_dc_math::validate_sample_type::<u8>(params.bit_depth)?;
    let intermediate_height = validate_subpel_params(params)?;
    let output_len = subpel_output_len(params, output_stride)?;
    if output.len() < output_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_len,
            actual: output.len(),
        });
    }
    if params.first_x > params.last_x || params.first_y > params.last_y {
        return Err(ReconError::SubpelReferenceBoundsInvalid {
            first_x: params.first_x,
            first_y: params.first_y,
            last_x: params.last_x,
            last_y: params.last_y,
        });
    }
    Ok(intermediate_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(bit_depth: BitDepth) -> SubpelPredictParams {
        SubpelPredictParams {
            interp: InterpolationFilter::EightTap,
            w: 2,
            h: 2,
            start_x: 1 << SCALE_SUBPEL_BITS,
            start_y: 1 << SCALE_SUBPEL_BITS,
            step_x: 1 << SCALE_SUBPEL_BITS,
            step_y: 1 << SCALE_SUBPEL_BITS,
            first_x: 0,
            first_y: 0,
            last_x: 3,
            last_y: 3,
            bit_depth,
        }
    }

    #[test]
    fn single_u8_rejects_bit_depth_and_inverted_bounds_before_mutation() -> Result<()> {
        let ten_bit_samples = [512u16; 16];
        let ten_bit_view = ReferencePlaneView::new(&ten_bit_samples, 4, 4)?;
        let sentinel = 0xa5;
        let mut output = [sentinel; 5];
        assert_eq!(
            subpel_predict_block_strided_into_u8(
                &ten_bit_view,
                &params(BitDepth::Ten),
                &mut output,
                3,
            ),
            Err(ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten,
            })
        );
        assert_eq!(output, [sentinel; 5]);

        let samples = [91u8; 16];
        let view = ReferencePlaneView::new(&samples, 4, 4)?;
        for invalid in [
            SubpelPredictParams {
                first_x: 2,
                last_x: 1,
                ..params(BitDepth::Eight)
            },
            SubpelPredictParams {
                first_y: 2,
                last_y: 1,
                ..params(BitDepth::Eight)
            },
        ] {
            assert_eq!(
                subpel_predict_block_strided_into_u8(&view, &invalid, &mut output, 3),
                Err(ReconError::SubpelReferenceBoundsInvalid {
                    first_x: invalid.first_x,
                    first_y: invalid.first_y,
                    last_x: invalid.last_x,
                    last_y: invalid.last_y,
                })
            );
            assert_eq!(output, [sentinel; 5]);
        }
        Ok(())
    }

    #[test]
    fn single_u8_fullpel_clamps_negative_bounds_before_index_conversion() -> Result<()> {
        let samples = (0..16u8).collect::<Vec<_>>();
        let view = ReferencePlaneView::new(&samples, 4, 4)?;
        let params = SubpelPredictParams {
            start_x: -2 * (1 << SCALE_SUBPEL_BITS),
            start_y: -2 * (1 << SCALE_SUBPEL_BITS),
            first_x: -2,
            first_y: -2,
            last_x: -1,
            last_y: -1,
            ..params(BitDepth::Eight)
        };
        let mut output = [u8::MAX; 4];

        subpel_predict_block_strided_into_u8(&view, &params, &mut output, params.w)?;

        assert_eq!(output, [samples[0]; 4]);
        Ok(())
    }

    #[test]
    fn single_u8_filtered_clamps_negative_bounds_before_index_conversion() -> Result<()> {
        let samples = (0..16u8).map(|value| value * 13).collect::<Vec<_>>();
        let view = ReferencePlaneView::new(&samples, 4, 4)?;
        let negative = SubpelPredictParams {
            interp: InterpolationFilter::EightTapSharp,
            start_x: -2 * (1 << SCALE_SUBPEL_BITS) + 5 * (1 << 6),
            start_y: -2 * (1 << SCALE_SUBPEL_BITS) + 7 * (1 << 6),
            step_x: 896,
            step_y: 1152,
            first_x: -2,
            first_y: -2,
            last_x: -1,
            last_y: -1,
            ..params(BitDepth::Eight)
        };
        let normalized = SubpelPredictParams {
            first_x: 0,
            first_y: 0,
            last_x: 0,
            last_y: 0,
            ..negative
        };
        let mut expected = [u16::MAX; 4];
        let mut actual = [u8::MAX; 4];

        subpel_predict_block_strided_into(&view, &normalized, &mut expected, normalized.w)?;
        subpel_predict_block_strided_into_u8(&view, &negative, &mut actual, negative.w)?;

        assert_eq!(actual.map(u16::from), expected);
        Ok(())
    }
}
