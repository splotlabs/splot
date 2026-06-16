// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
//
// Fuzz target: source-backed intra prediction and current-frame workspace
// primitives must return typed results, never panic, for bounded structured
// inputs derived from arbitrary bytes. This target intentionally does not parse
// AV2 bitstreams, invoke splot-decode, write filesystem paths, or invoke
// AVM/dav2d. Run with:
//
//     cargo install cargo-fuzz --locked
//     cargo +nightly fuzz run recon_intra_prediction_bytes
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, IntraCardinalDirection,
    IntraCardinalEdges, IntraDcEdges, IntraDirectionalAngle, IntraDirectionalAngleEdges,
    IntraMiddleDirectionalAngle, IntraMiddleDirectionalAngleEdges, IntraPaethEdges,
    IntraRectBlockSize, IntraSmoothEdges, IntraSmoothMode, IntraSquareBlockSize, OutputIndex,
    PixelFormat, PlaneId, PlaneRect, PlaneSize, ReconSample, apply_intra_ibp_dc_rect,
    predict_intra_cardinal_directional_rect_into, predict_intra_dc_rect_into,
    predict_intra_dc_rect_value, predict_intra_dc_square, predict_intra_dc_square_into,
    predict_intra_dc_square_value,
    predict_intra_dc_subsampled_rect_into, predict_intra_dc_subsampled_rect_value,
    predict_intra_directional_angle_rect_from_p_angle_into,
    predict_intra_middle_directional_angle_rect_from_p_angle_into, predict_intra_paeth_rect_into,
    predict_intra_smooth_rect_into,
};

const MIN_BLOCK_LOG2: u8 = 2;
const BLOCK_LOG2_SPAN: u8 = 5;
const MAX_STRIDE_PADDING: usize = 8;
const MAX_WORKSPACE_SIDE: usize = 80;
const MAX_OPERATIONS: usize = 8;

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    let selector = input.byte();
    let bit_depth = if selector & 0b0000_0001 == 0 {
        BitDepth::Eight
    } else {
        BitDepth::Ten
    };

    match (bit_depth, selector & 0b0000_0010 != 0) {
        (BitDepth::Eight, true) => run_case::<u16>(&mut input, bit_depth),
        (BitDepth::Eight, false) => run_case::<u8>(&mut input, bit_depth),
        (BitDepth::Ten, _) => run_case::<u16>(&mut input, bit_depth),
    }
});

fn run_case<T: ReconSample>(input: &mut FuzzInput<'_>, bit_depth: BitDepth) {
    let operations = 1 + usize::from(input.byte()) % MAX_OPERATIONS;
    for _ in 0..operations {
        match input.byte() % 9 {
            0 => run_direct_dc_case::<T>(input, bit_depth),
            1 => run_direct_paeth_case::<T>(input, bit_depth),
            2 => run_direct_smooth_case::<T>(input, bit_depth),
            3 => run_direct_cardinal_case::<T>(input, bit_depth),
            4 => run_direct_ibp_dc_case::<T>(input, bit_depth),
            5 => run_direct_directional_angle_case::<T>(input, bit_depth),
            6 => run_direct_middle_directional_angle_case::<T>(input, bit_depth),
            7 => run_interior_workspace_case::<T>(input, bit_depth),
            _ => run_random_workspace_case::<T>(input, bit_depth),
        }
    }
}

fn run_direct_dc_case<T: ReconSample>(input: &mut FuzzInput<'_>, bit_depth: BitDepth) {
    let Some(rect_size) = rect_size_from_input(input) else {
        return;
    };
    let Some(square_size) = square_size_from_input(input) else {
        return;
    };

    let edge_selector = input.byte();
    let Some(left) = samples_from_input::<T>(input, rect_size.height(), bit_depth) else {
        return;
    };
    let Some(above) = samples_from_input::<T>(input, rect_size.width(), bit_depth) else {
        return;
    };
    let left_len = maybe_short_len(rect_size.height(), edge_selector & 0b0001_0000 != 0);
    let above_len = maybe_short_len(rect_size.width(), edge_selector & 0b0010_0000 != 0);
    let edges = dc_edges(edge_selector, &left[..left_len], &above[..above_len]);
    let _ = predict_intra_dc_rect_value(bit_depth, rect_size, edges);

    let Some(mut rect_output) = output_buffer::<T>(input, rect_size, bit_depth) else {
        return;
    };
    let rect_stride = output_stride(input, rect_size.width());
    let edges = dc_edges(edge_selector, &left[..left_len], &above[..above_len]);
    let _ = predict_intra_dc_rect_into(bit_depth, rect_size, edges, &mut rect_output, rect_stride);

    let edges = dc_edges(edge_selector, &left[..left_len], &above[..above_len]);
    let _ = predict_intra_dc_subsampled_rect_value(bit_depth, rect_size, edges);
    let Some(mut subsampled_output) = output_buffer::<T>(input, rect_size, bit_depth) else {
        return;
    };
    let subsampled_stride = output_stride(input, rect_size.width());
    let edges = dc_edges(edge_selector, &left[..left_len], &above[..above_len]);
    let _ = predict_intra_dc_subsampled_rect_into(
        bit_depth,
        rect_size,
        edges,
        &mut subsampled_output,
        subsampled_stride,
    );

    let Some(square_left) = samples_from_input::<T>(input, square_size.side_len(), bit_depth)
    else {
        return;
    };
    let Some(square_above) = samples_from_input::<T>(input, square_size.side_len(), bit_depth)
    else {
        return;
    };
    let square_edges = dc_edges(edge_selector, &square_left, &square_above);
    let _ = predict_intra_dc_square_value(bit_depth, square_size, square_edges);
    let square_edges = dc_edges(edge_selector, &square_left, &square_above);
    let _ = predict_intra_dc_square(bit_depth, square_size, square_edges);

    let Some(mut square_output) = output_buffer::<T>(input, square_size.into(), bit_depth) else {
        return;
    };
    let square_stride = output_stride(input, square_size.side_len());
    let square_edges = dc_edges(edge_selector, &square_left, &square_above);
    let _ = predict_intra_dc_square_into(
        bit_depth,
        square_size,
        square_edges,
        &mut square_output,
        square_stride,
    );
}

fn run_direct_paeth_case<T: ReconSample>(input: &mut FuzzInput<'_>, bit_depth: BitDepth) {
    let Some(size) = rect_size_from_input(input) else {
        return;
    };
    let selector = input.byte();
    let Some(left) = samples_from_input::<T>(input, size.height(), bit_depth) else {
        return;
    };
    let Some(above) = samples_from_input::<T>(input, size.width(), bit_depth) else {
        return;
    };
    let Some(top_left) = sample_from_input::<T>(input, bit_depth) else {
        return;
    };
    let Some(mut output) = output_buffer::<T>(input, size, bit_depth) else {
        return;
    };

    let left_len = maybe_short_len(size.height(), selector & 0b0000_0001 != 0);
    let above_len = maybe_short_len(size.width(), selector & 0b0000_0010 != 0);
    let stride = output_stride(input, size.width());
    let edges = IntraPaethEdges::new(&left[..left_len], &above[..above_len], top_left);
    let _ = predict_intra_paeth_rect_into(bit_depth, size, edges, &mut output, stride);
}

fn run_direct_smooth_case<T: ReconSample>(input: &mut FuzzInput<'_>, bit_depth: BitDepth) {
    let Some(size) = rect_size_from_input(input) else {
        return;
    };
    let selector = input.byte();
    let Some(left) = samples_from_input::<T>(input, size.height() + 1, bit_depth) else {
        return;
    };
    let Some(above) = samples_from_input::<T>(input, size.width() + 1, bit_depth) else {
        return;
    };
    let Some(mut output) = output_buffer::<T>(input, size, bit_depth) else {
        return;
    };

    let left_len = maybe_short_len(size.height() + 1, selector & 0b0000_0001 != 0);
    let above_len = maybe_short_len(size.width() + 1, selector & 0b0000_0010 != 0);
    let stride = output_stride(input, size.width());
    let mode = smooth_mode(selector);
    let edges = IntraSmoothEdges::new(&left[..left_len], &above[..above_len]);
    let _ = predict_intra_smooth_rect_into(bit_depth, size, mode, edges, &mut output, stride);
}

fn run_direct_cardinal_case<T: ReconSample>(input: &mut FuzzInput<'_>, bit_depth: BitDepth) {
    let Some(size) = rect_size_from_input(input) else {
        return;
    };
    let selector = input.byte();
    let Some(left) = samples_from_input::<T>(input, size.height(), bit_depth) else {
        return;
    };
    let Some(above) = samples_from_input::<T>(input, size.width(), bit_depth) else {
        return;
    };
    let Some(mut output) = output_buffer::<T>(input, size, bit_depth) else {
        return;
    };

    let left_len = maybe_short_len(size.height(), selector & 0b0000_0001 != 0);
    let above_len = maybe_short_len(size.width(), selector & 0b0000_0010 != 0);
    let direction = cardinal_direction(selector);
    let edges = cardinal_edges(selector, &left[..left_len], &above[..above_len]);
    let stride = output_stride(input, size.width());
    let _ = predict_intra_cardinal_directional_rect_into(
        bit_depth,
        size,
        direction,
        edges,
        &mut output,
        stride,
    );
}

fn run_direct_ibp_dc_case<T: ReconSample>(input: &mut FuzzInput<'_>, bit_depth: BitDepth) {
    let Some(size) = rect_size_from_input(input) else {
        return;
    };
    let selector = input.byte();
    let Some(left) = samples_from_input::<T>(input, size.height(), bit_depth) else {
        return;
    };
    let Some(above) = samples_from_input::<T>(input, size.width(), bit_depth) else {
        return;
    };
    let Some(mut output) = output_buffer::<T>(input, size, bit_depth) else {
        return;
    };

    let left_len = maybe_short_len(size.height(), selector & 0b0000_0001 != 0);
    let above_len = maybe_short_len(size.width(), selector & 0b0000_0010 != 0);
    let edges = dc_edges(selector, &left[..left_len], &above[..above_len]);
    let stride = output_stride(input, size.width());
    let _ = apply_intra_ibp_dc_rect(bit_depth, size, edges, &mut output, stride);
}

fn run_direct_directional_angle_case<T: ReconSample>(
    input: &mut FuzzInput<'_>,
    bit_depth: BitDepth,
) {
    let Some(size) = rect_size_from_input(input) else {
        return;
    };
    let selector = input.byte();
    let edge_len = size.width() + size.height();
    let Some(mut left) = samples_from_input::<T>(input, edge_len, bit_depth) else {
        return;
    };
    let Some(mut above) = samples_from_input::<T>(input, edge_len, bit_depth) else {
        return;
    };
    let Some(mut output) = output_buffer::<T>(input, size, bit_depth) else {
        return;
    };

    if selector & 0b0100_0000 != 0 {
        if let Some(invalid) = invalid_sample::<T>(bit_depth) {
            if selector & 0b0000_0001 == 0 {
                left[edge_len - 1] = invalid;
            } else {
                above[edge_len - 1] = invalid;
            }
        }
    }

    let left_len = maybe_short_len(edge_len, selector & 0b0000_0010 != 0);
    let above_len = maybe_short_len(edge_len, selector & 0b0000_0100 != 0);
    let edges = directional_angle_edges(selector, &left[..left_len], &above[..above_len]);
    let stride = output_stride(input, size.width());
    let p_angle = directional_angle_pangle(selector);
    let _ = predict_intra_directional_angle_rect_from_p_angle_into(
        bit_depth,
        size,
        p_angle,
        edges,
        &mut output,
        stride,
    );
}

fn run_direct_middle_directional_angle_case<T: ReconSample>(
    input: &mut FuzzInput<'_>,
    bit_depth: BitDepth,
) {
    let Some(size) = rect_size_from_input(input) else {
        return;
    };
    let selector = input.byte();
    let p_angle_selector = input.byte();
    let left_edge_len = size.height() + 1;
    let above_edge_len = size.width() + 1;
    let Some(mut left) = samples_from_input::<T>(input, left_edge_len, bit_depth) else {
        return;
    };
    let Some(mut above) = samples_from_input::<T>(input, above_edge_len, bit_depth) else {
        return;
    };
    let Some(mut output) = output_buffer::<T>(input, size, bit_depth) else {
        return;
    };

    if selector & 0b0100_0000 != 0 {
        if let Some(invalid) = invalid_sample::<T>(bit_depth) {
            if selector & 0b0000_0001 == 0 {
                left[0] = invalid;
            } else {
                above[0] = invalid;
            }
        }
    }

    let left_len = maybe_short_len(left_edge_len, selector & 0b0000_0010 != 0);
    let above_len = maybe_short_len(above_edge_len, selector & 0b0000_0100 != 0);
    let edges = middle_directional_angle_edges(selector, &left[..left_len], &above[..above_len]);
    let stride = output_stride(input, size.width());
    let p_angle = middle_directional_angle_pangle(p_angle_selector);
    let _ = predict_intra_middle_directional_angle_rect_from_p_angle_into(
        bit_depth,
        size,
        p_angle,
        edges,
        &mut output,
        stride,
    );
}

fn run_interior_workspace_case<T: ReconSample>(input: &mut FuzzInput<'_>, bit_depth: BitDepth) {
    let Some(size) = rect_size_from_input(input) else {
        return;
    };
    let edge_len = size.width() + size.height();
    let width = edge_len + 2;
    let height = edge_len + 2;
    let Some(mut workspace) =
        workspace_from_dimensions::<T>(input, bit_depth, PixelFormat::Yuv444, width, height)
    else {
        return;
    };
    let Some(rect) = PlaneRect::new(0, 0, width, height).ok() else {
        return;
    };
    let Some(samples) = samples_from_input::<T>(input, width * height, bit_depth) else {
        return;
    };
    let _ = workspace.write_rect(PlaneId::Y, rect, &samples, width);

    if let Ok(edges) = workspace.intra_dc_edges_for_rect(PlaneId::Y, 1, 1, size) {
        let _ = predict_intra_dc_rect_value(bit_depth, size, edges.as_dc_edges());
    }
    let _ = workspace.predict_intra_dc_rect(PlaneId::Y, 1, 1, size);
    let _ = workspace.predict_intra_dc_subsampled_rect(PlaneId::Y, 1, 1, size);
    let _ = workspace.predict_intra_ibp_dc_rect(PlaneId::Y, 1, 1, size);
    let _ = workspace.predict_intra_paeth_rect(PlaneId::Y, 1, 1, size);
    let _ = workspace.predict_intra_smooth_rect(PlaneId::Y, 1, 1, size, smooth_mode(input.byte()));
    let _ = workspace.predict_intra_cardinal_directional_rect(
        PlaneId::Y,
        1,
        1,
        size,
        cardinal_direction(input.byte()),
    );
    let _ = workspace.predict_intra_directional_angle_rect(
        PlaneId::U,
        1,
        1,
        size,
        directional_angle(input.byte()),
    );
    let _ = workspace.predict_intra_middle_directional_angle_rect(
        PlaneId::U,
        1,
        1,
        size,
        middle_directional_angle(input.byte()),
    );
    let _ = workspace.freeze();
}

fn run_random_workspace_case<T: ReconSample>(input: &mut FuzzInput<'_>, bit_depth: BitDepth) {
    let pixel_format = pixel_format_from_input(input);
    let width = 1 + usize::from(input.byte()) % MAX_WORKSPACE_SIDE;
    let height = 1 + usize::from(input.byte()) % MAX_WORKSPACE_SIDE;
    let Some(mut workspace) =
        workspace_from_dimensions::<T>(input, bit_depth, pixel_format, width, height)
    else {
        return;
    };
    let Some(size) = rect_size_from_input(input) else {
        return;
    };
    let plane = plane_from_input(input);
    let x = usize::from(input.byte()) % (width + 4);
    let y = usize::from(input.byte()) % (height + 4);
    let Some(rect) = PlaneRect::new(x, y, size.width(), size.height()).ok() else {
        return;
    };
    let Some(fill) = sample_from_input::<T>(input, bit_depth) else {
        return;
    };
    let _ = workspace.fill_rect(plane, rect, fill);

    let source_stride = output_stride(input, size.width());
    let source_len = row_strided_len(size, source_stride);
    let Some(source) = samples_from_input::<T>(input, source_len, bit_depth) else {
        return;
    };
    let _ = workspace.write_rect(plane, rect, &source, source_stride);

    let _ = workspace.intra_dc_edges_for_rect(plane, x, y, size);
    let _ = workspace.predict_intra_dc_rect(plane, x, y, size);
    let _ = workspace.predict_intra_dc_subsampled_rect(plane, x, y, size);
    let _ = workspace.predict_intra_ibp_dc_rect(plane, x, y, size);
    let _ = workspace.predict_intra_paeth_rect(plane, x, y, size);
    let _ = workspace.predict_intra_smooth_rect(plane, x, y, size, smooth_mode(input.byte()));
    let _ =
        workspace.predict_intra_cardinal_directional_rect(plane, x, y, size, cardinal_direction(input.byte()));
    let _ = workspace.predict_intra_directional_angle_rect(
        plane,
        x,
        y,
        size,
        directional_angle(input.byte()),
    );
    let _ = workspace.predict_intra_middle_directional_angle_rect(
        plane,
        x,
        y,
        size,
        middle_directional_angle(input.byte()),
    );
    let _ = workspace.rect_rows(plane, rect).map(|rows| {
        let _: usize = rows.map(<[T]>::len).sum();
    });
    let _ = workspace.samples(plane).map(<[T]>::len);
    let _ = workspace.freeze();
}

fn workspace_from_dimensions<T: ReconSample>(
    input: &mut FuzzInput<'_>,
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
    width: usize,
    height: usize,
) -> Option<CurrentFrameWorkspace<T>> {
    let coded = PlaneSize::new(width, height).ok()?;
    let visible = PlaneRect::new(0, 0, width, height).ok()?;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(u64::from(input.byte())),
        bit_depth,
        pixel_format,
        coded,
        visible,
    )
    .ok()?;
    let fill = sample_from_input::<T>(input, bit_depth)?;
    CurrentFrameWorkspace::new(info, fill).ok()
}

fn rect_size_from_input(input: &mut FuzzInput<'_>) -> Option<IntraRectBlockSize> {
    let log2_width = MIN_BLOCK_LOG2 + input.byte() % BLOCK_LOG2_SPAN;
    let log2_height = MIN_BLOCK_LOG2 + input.byte() % BLOCK_LOG2_SPAN;
    IntraRectBlockSize::new(log2_width, log2_height).ok()
}

fn square_size_from_input(input: &mut FuzzInput<'_>) -> Option<IntraSquareBlockSize> {
    let log2_size = MIN_BLOCK_LOG2 + input.byte() % BLOCK_LOG2_SPAN;
    IntraSquareBlockSize::new(log2_size).ok()
}

fn dc_edges<'a, T: ReconSample>(
    selector: u8,
    left: &'a [T],
    above: &'a [T],
) -> IntraDcEdges<'a, T> {
    match selector & 0b0000_0011 {
        0 => IntraDcEdges::none(),
        1 => IntraDcEdges::left(left),
        2 => IntraDcEdges::above(above),
        _ => IntraDcEdges::both(left, above),
    }
}

fn cardinal_edges<'a, T: ReconSample>(
    selector: u8,
    left: &'a [T],
    above: &'a [T],
) -> IntraCardinalEdges<'a, T> {
    match (selector >> 2) & 0b0000_0011 {
        0 => IntraCardinalEdges::new(None, None),
        1 => IntraCardinalEdges::left(left),
        2 => IntraCardinalEdges::above(above),
        _ => IntraCardinalEdges::both(left, above),
    }
}

fn directional_angle_edges<'a, T: ReconSample>(
    selector: u8,
    left: &'a [T],
    above: &'a [T],
) -> IntraDirectionalAngleEdges<'a, T> {
    match (selector >> 3) & 0b0000_0011 {
        0 => IntraDirectionalAngleEdges::new(None, None),
        1 => IntraDirectionalAngleEdges::left(left),
        2 => IntraDirectionalAngleEdges::above(above),
        _ => IntraDirectionalAngleEdges::both(left, above),
    }
}

fn middle_directional_angle_edges<'a, T: ReconSample>(
    selector: u8,
    left: &'a [T],
    above: &'a [T],
) -> IntraMiddleDirectionalAngleEdges<'a, T> {
    match (selector >> 3) & 0b0000_0011 {
        0 => IntraMiddleDirectionalAngleEdges::new(None, None),
        1 => IntraMiddleDirectionalAngleEdges::new(Some(left), None),
        2 => IntraMiddleDirectionalAngleEdges::new(None, Some(above)),
        _ => IntraMiddleDirectionalAngleEdges::both(left, above),
    }
}

fn output_buffer<T: ReconSample>(
    input: &mut FuzzInput<'_>,
    size: IntraRectBlockSize,
    bit_depth: BitDepth,
) -> Option<Vec<T>> {
    let stride = size.width() + usize::from(input.byte()) % (MAX_STRIDE_PADDING + 1);
    let required = row_strided_len(size, stride);
    let mut samples = Vec::new();
    samples.try_reserve_exact(required).ok()?;
    for _ in 0..required {
        samples.push(sample_from_input::<T>(input, bit_depth)?);
    }
    Some(samples)
}

fn row_strided_len(size: IntraRectBlockSize, stride: usize) -> usize {
    (size.height() - 1) * stride + size.width()
}

fn output_stride(input: &mut FuzzInput<'_>, width: usize) -> usize {
    if input.byte() & 0b0000_0001 == 0 {
        width + usize::from(input.byte()) % (MAX_STRIDE_PADDING + 1)
    } else {
        width.saturating_sub(1)
    }
}

fn samples_from_input<T: ReconSample>(
    input: &mut FuzzInput<'_>,
    len: usize,
    bit_depth: BitDepth,
) -> Option<Vec<T>> {
    let mut samples = Vec::new();
    samples.try_reserve_exact(len).ok()?;
    for _ in 0..len {
        samples.push(sample_from_input::<T>(input, bit_depth)?);
    }
    Some(samples)
}

fn sample_from_input<T: ReconSample>(input: &mut FuzzInput<'_>, bit_depth: BitDepth) -> Option<T> {
    let value = match bit_depth {
        BitDepth::Eight => u16::from(input.byte()),
        BitDepth::Ten => {
            let high = u16::from(input.byte());
            let low = u16::from(input.byte() & 0b0000_0011);
            ((high << 2) | low) & bit_depth.max_sample()
        }
    };
    T::try_from_u16(value).ok()
}

fn invalid_sample<T: ReconSample>(bit_depth: BitDepth) -> Option<T> {
    bit_depth
        .max_sample()
        .checked_add(1)
        .and_then(|value| T::try_from_u16(value).ok())
}

fn maybe_short_len(expected: usize, shorten: bool) -> usize {
    if shorten {
        expected.saturating_sub(1)
    } else {
        expected
    }
}

const fn smooth_mode(selector: u8) -> IntraSmoothMode {
    match selector % 3 {
        0 => IntraSmoothMode::Smooth,
        1 => IntraSmoothMode::SmoothVertical,
        _ => IntraSmoothMode::SmoothHorizontal,
    }
}

const fn cardinal_direction(selector: u8) -> IntraCardinalDirection {
    if selector & 0b1000_0000 == 0 {
        IntraCardinalDirection::Vertical
    } else {
        IntraCardinalDirection::Horizontal
    }
}

const fn directional_angle(selector: u8) -> IntraDirectionalAngle {
    match selector % 3 {
        0 => IntraDirectionalAngle::D45,
        1 => IntraDirectionalAngle::D67,
        _ => IntraDirectionalAngle::D203,
    }
}

const fn middle_directional_angle(selector: u8) -> IntraMiddleDirectionalAngle {
    match selector % 3 {
        0 => IntraMiddleDirectionalAngle::D113,
        1 => IntraMiddleDirectionalAngle::D135,
        _ => IntraMiddleDirectionalAngle::D157,
    }
}

const fn directional_angle_pangle(selector: u8) -> u16 {
    match selector % 10 {
        0 => 45,
        1 => 67,
        2 => 203,
        3 => 0,
        4 => 90,
        5 => 113,
        6 => 135,
        7 => 157,
        8 => 180,
        _ => 270,
    }
}

const fn middle_directional_angle_pangle(selector: u8) -> u16 {
    match selector % 10 {
        0 => 113,
        1 => 135,
        2 => 157,
        3 => 45,
        4 => 67,
        5 => 90,
        6 => 180,
        7 => 203,
        8 => 270,
        _ => 0,
    }
}

fn pixel_format_from_input(input: &mut FuzzInput<'_>) -> PixelFormat {
    match input.byte() % 4 {
        0 => PixelFormat::Monochrome,
        1 => PixelFormat::Yuv420,
        2 => PixelFormat::Yuv422,
        _ => PixelFormat::Yuv444,
    }
}

fn plane_from_input(input: &mut FuzzInput<'_>) -> PlaneId {
    match input.byte() % 3 {
        0 => PlaneId::Y,
        1 => PlaneId::U,
        _ => PlaneId::V,
    }
}

struct FuzzInput<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> FuzzInput<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn byte(&mut self) -> u8 {
        let byte = self.bytes.get(self.offset).copied().unwrap_or(0);
        self.offset = self.offset.saturating_add(1);
        byte
    }
}
