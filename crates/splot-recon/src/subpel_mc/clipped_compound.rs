// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

pub(super) fn two_axis<T: ReconSample>(
    references: [&ReferencePlaneView<'_, T>; 2],
    params: [&SubpelPredictParams; 2],
    sources: [&[u16]; 2],
    cwp_weight: i16,
    scratch: &mut [i16],
    output: &mut [u16],
    output_stride: usize,
) {
    match params[0].w {
        4 => two_axis_lanes::<4, T>(
            references,
            params,
            sources,
            cwp_weight,
            scratch,
            output,
            output_stride,
        ),
        8 => two_axis_lanes::<8, T>(
            references,
            params,
            sources,
            cwp_weight,
            scratch,
            output,
            output_stride,
        ),
        _ => (),
    }
}

fn two_axis_lanes<const LANES: usize, T: ReconSample>(
    references: [&ReferencePlaneView<'_, T>; 2],
    params: [&SubpelPredictParams; 2],
    sources: [&[u16]; 2],
    cwp_weight: i16,
    scratch: &mut [i16],
    output: &mut [u16],
    output_stride: usize,
) {
    const MAX_INTERMEDIATE: usize = (8 + NUM_TAPS - 1) * 8;
    let (first, second) = scratch.split_at_mut(MAX_INTERMEDIATE);
    let intermediate = [first, second];
    let horizontal = params.map(|params| {
        let filter = params.interp.pass_index(params.w as u32) as usize;
        let phase = ((params.start_x >> 6) & SUBPEL_MASK) as usize;
        let (start, end) = ACTIVE_TAP_SPANS[filter][phase];
        (&SUBPEL_FILTERS[filter][phase][start..end], start)
    });
    for reference in 0..2 {
        let (taps, tap_start) = horizontal[reference];
        let first_x = (params[reference].start_x >> SCALE_SUBPEL_BITS) + tap_start as i32 - 3;
        let window_len = LANES + taps.len() - 1;
        let last_x = first_x + window_len as i32 - 1;
        let in_range = first_x >= params[reference].first_x
            && last_x <= params[reference].last_x
            && first_x >= 0
            && last_x < references[reference].width as i32;
        if in_range {
            for row in 0..params[reference].h + NUM_TAPS - 1 {
                let source_row = ((params[reference].start_y >> SCALE_SUBPEL_BITS) + row as i32 - 3)
                    .clamp(params[reference].first_y, params[reference].last_y)
                    as usize;
                let row_start =
                    source_row.min(references[reference].height - 1) * references[reference].stride;
                let start = row_start + first_x as usize;
                let source = &sources[reference][start..start + window_len];
                let lanes = clipped_horizontal_filter::<LANES>(source, taps);
                intermediate[reference][row * LANES..(row + 1) * LANES].copy_from_slice(&lanes); // splot-copy-ok: store clipped horizontal SIMD lanes in caller scratch
            }
            continue;
        }
        let physical_last = references[reference].width as i32 - 1;
        let bounded_first = params[reference].first_x.clamp(0, physical_last);
        let bounded_last = params[reference].last_x.clamp(0, physical_last);
        let copy_first = first_x.max(bounded_first);
        let copy_last = last_x.min(bounded_last);
        let copy_len = if copy_first <= copy_last {
            (copy_last - copy_first) as usize + 1
        } else {
            0
        };
        let prefix_len = if copy_len == 0 {
            window_len
        } else {
            (copy_first - first_x) as usize
        };
        for row in 0..params[reference].h + NUM_TAPS - 1 {
            let source_row = ((params[reference].start_y >> SCALE_SUBPEL_BITS) + row as i32 - 3)
                .clamp(params[reference].first_y, params[reference].last_y)
                as usize;
            let row_start =
                source_row.min(references[reference].height - 1) * references[reference].stride;
            let mut window = [0u16; 8 + NUM_TAPS - 1];
            if copy_len == 0 {
                let source_column = first_x.clamp(bounded_first, bounded_last) as usize;
                window[..window_len].fill(sources[reference][row_start + source_column]);
            } else {
                let source_start = row_start + copy_first as usize;
                let source_end = source_start + copy_len;
                let copied_end = prefix_len + copy_len;
                window[..prefix_len].fill(sources[reference][source_start]);
                window[prefix_len..copied_end]
                    .copy_from_slice(&sources[reference][source_start..source_end]); // splot-copy-ok: materialize one clamped SIMD row window
                window[copied_end..window_len].fill(sources[reference][source_end - 1]);
            }
            let lanes = clipped_horizontal_filter::<LANES>(&window[..window_len], taps);
            intermediate[reference][row * LANES..(row + 1) * LANES].copy_from_slice(&lanes); // splot-copy-ok: store clipped horizontal SIMD lanes in caller scratch
        }
    }
    finish_fused_compound_2d!(
        LANES,
        params,
        cwp_weight,
        intermediate,
        output,
        output_stride
    );
}

#[allow(clippy::inline_always, reason = "measured clipped compound hot path")]
#[inline(always)]
fn clipped_horizontal_filter<const LANES: usize>(window: &[u16], taps: &[i32]) -> [i16; LANES] {
    let mut sum = Simd::<i32, LANES>::splat(0);
    for (samples, &tap) in window.windows(LANES).zip(taps) {
        sum = tap_mac(sum, Simd::<u16, LANES>::from_slice(samples).cast(), tap);
    }
    round2_simd(sum, INTER_ROUND0).cast::<i16>().to_array()
}

pub(super) fn horizontal<T: ReconSample>(
    references: [&ReferencePlaneView<'_, T>; 2],
    params: [&SubpelPredictParams; 2],
    sources: [&[u16]; 2],
    cwp_weight: i16,
    output: &mut [u16],
    output_stride: usize,
) {
    let filters = params.map(|params| {
        let filter = params.interp.pass_index(params.w as u32) as usize;
        let phase = ((params.start_x >> 6) & SUBPEL_MASK) as usize;
        let (start, end) = ACTIVE_TAP_SPANS[filter][phase];
        (&SUBPEL_FILTERS[filter][phase][start..end], start)
    });
    let forward = i32::from(cwp_weight);
    let backward = 16 - forward;
    let max_sample = i32::from(params[0].bit_depth.max_sample());
    let y0 = params.map(|params| params.start_y >> SCALE_SUBPEL_BITS);
    for row in 0..params[0].h {
        let source_rows: [usize; 2] = core::array::from_fn(|reference| {
            (y0[reference] + row as i32).clamp(params[reference].first_y, params[reference].last_y)
                as usize
        });
        let destination = &mut output[row * output_stride..][..params[0].w];
        let vector_width8 = params[0].w - params[0].w % 8;
        for col in (0..vector_width8).step_by(8) {
            let predictors =
                predictors::<8, T>(references, params, sources, filters, source_rows, col);
            let blended = round2_simd(
                predictors[0] * Simd::splat(forward) + predictors[1] * Simd::splat(backward),
                4 + compound_inter_post_round(),
            )
            .simd_clamp(Simd::splat(0), Simd::splat(max_sample))
            .cast::<u16>();
            destination[col..col + 8].copy_from_slice(&blended.to_array()); // splot-copy-ok: publish clipped horizontal compound lanes
        }
        let vector_width4 = params[0].w - params[0].w % 4;
        for col in (vector_width8..vector_width4).step_by(4) {
            let predictors =
                predictors::<4, T>(references, params, sources, filters, source_rows, col);
            let blended = round2_simd(
                predictors[0] * Simd::splat(forward) + predictors[1] * Simd::splat(backward),
                4 + compound_inter_post_round(),
            )
            .simd_clamp(Simd::splat(0), Simd::splat(max_sample))
            .cast::<u16>();
            destination[col..col + 4].copy_from_slice(&blended.to_array()); // splot-copy-ok: publish clipped horizontal compound lanes
        }
        for (offset, slot) in destination[vector_width4..].iter_mut().enumerate() {
            let col = vector_width4 + offset;
            let predictors =
                predictors::<1, T>(references, params, sources, filters, source_rows, col);
            *slot = round2_i32(
                forward * predictors[0][0] + backward * predictors[1][0],
                4 + compound_inter_post_round(),
            )
            .clamp(0, max_sample) as u16;
        }
    }
}

#[inline]
fn predictors<const LANES: usize, T: ReconSample>(
    references: [&ReferencePlaneView<'_, T>; 2],
    params: [&SubpelPredictParams; 2],
    sources: [&[u16]; 2],
    filters: [(&[i32], usize); 2],
    rows: [usize; 2],
    col: usize,
) -> [Simd<i32, LANES>; 2] {
    core::array::from_fn(|reference| {
        let (taps, tap_start) = filters[reference];
        let mut sum = Simd::splat(0);
        for (tap_offset, &tap) in taps.iter().enumerate() {
            sum = tap_mac(
                sum,
                gather::<LANES, T>(
                    references[reference],
                    params[reference],
                    sources[reference],
                    rows[reference],
                    col,
                    tap_start + tap_offset,
                )
                .cast(),
                tap,
            );
        }
        round2_simd(sum, INTER_ROUND0)
    })
}

#[inline]
fn gather<const LANES: usize, T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    source: &[u16],
    row: usize,
    col: usize,
    tap: usize,
) -> Simd<u16, LANES> {
    let first = (params.start_x >> SCALE_SUBPEL_BITS) + col as i32 + tap as i32 - 3;
    let last = first + LANES as i32 - 1;
    let row_start = row.min(reference.height - 1) * reference.stride;
    if first >= params.first_x
        && last <= params.last_x
        && first >= 0
        && last < reference.width as i32
    {
        return Simd::from_slice(&source[row_start + first as usize..]);
    }
    Simd::from_array(core::array::from_fn(|lane| {
        let source_column = (first + lane as i32)
            .clamp(params.first_x, params.last_x)
            .clamp(0, reference.width as i32 - 1) as usize;
        source[row_start + source_column]
    }))
}
