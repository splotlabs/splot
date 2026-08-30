// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_encode::{
    BitDepth, ChromaSubsampling, Error, Frame, FrameId, FrameInfo, FramePlaneInput,
    FramePlanesInput, FrameTimestamp, RetainedFrame,
};
use splot_recon::{
    BitDepth as ReconBitDepth, DecodedFrame, DecodedFrameInfo, FramePlanes, OutputIndex,
    PixelFormat, Plane, PlaneRect, PlaneSize, SharedFrame,
};

const MAX_LUMA_WIDTH: usize = 16;
const MAX_LUMA_HEIGHT: usize = 16;
const MAX_STRIDE_PADDING: usize = 3;

fuzz_target!(|data: &[u8]| {
    exercise_fixed_cases();

    let mut input = FuzzInput::new(data);
    let width = input.bounded_usize(MAX_LUMA_WIDTH) + 1;
    let height = input.bounded_usize(MAX_LUMA_HEIGHT) + 1;
    let luma_size = size(width, height);
    let chroma_size = PixelFormat::Yuv420
        .chroma_size(luma_size)
        .unwrap_or_else(|err| panic!("bounded chroma size should not overflow: {err:?}"))
        .unwrap_or_else(|| panic!("YUV420 must have chroma planes"));

    let bit_depth = match input.byte() & 0b0000_0011 {
        0 => BitDepth::Eight,
        1 => BitDepth::Ten,
        2 => BitDepth::Twelve,
        _ => BitDepth::Eight,
    };
    let chroma = match input.byte() & 0b0000_0011 {
        0 => ChromaSubsampling::Yuv420,
        1 => ChromaSubsampling::Monochrome,
        2 => ChromaSubsampling::Yuv422,
        _ => ChromaSubsampling::Yuv444,
    };
    let info = FrameInfo::new(
        FrameId::new(u64::from(input.byte())),
        luma_size,
        bit_depth,
        chroma,
    )
    .with_timestamp(FrameTimestamp::new(i64::from(input.byte())));

    let flags = input.byte();
    let y_stride = stride(width, flags & 0b0000_0001 != 0, &mut input);
    let uv_stride = stride(chroma_size.width(), flags & 0b0000_0010 != 0, &mut input);
    let y_len = maybe_truncated_len(
        required_len(width, height, y_stride),
        flags & 0b0000_0100 != 0,
    );
    let u_len = maybe_truncated_len(
        required_len(chroma_size.width(), chroma_size.height(), uv_stride),
        flags & 0b0000_1000 != 0,
    );
    let v_len = maybe_truncated_len(
        required_len(chroma_size.width(), chroma_size.height(), uv_stride),
        flags & 0b0001_0000 != 0,
    );

    let y = vec![input.byte(); y_len];
    let u = vec![input.byte(); u_len];
    let v = vec![input.byte(); v_len];

    let y_input = FramePlaneInput::new(&y, y_stride, rect(0, 0, width, height));
    let u_input = FramePlaneInput::new(
        &u,
        uv_stride,
        rect(0, 0, chroma_size.width(), chroma_size.height()),
    );
    let v_input = FramePlaneInput::new(
        &v,
        uv_stride,
        rect(0, 0, chroma_size.width(), chroma_size.height()),
    );

    let u_present = flags & 0b0010_0000 == 0;
    let v_present = flags & 0b0100_0000 == 0;
    let planes = FramePlanesInput::new(
        y_input,
        u_present.then_some(u_input),
        v_present.then_some(v_input),
    );

    match Frame::from_planes(info, planes) {
        Ok(frame) => assert_valid_frame(frame, width, height, &y, &u, &v),
        Err(Error::UnsupportedInputBitDepth { .. })
        | Err(Error::UnsupportedInputChromaSubsampling { .. })
        | Err(Error::MissingInputPlane { .. })
        | Err(Error::UnexpectedInputPlane { .. })
        | Err(Error::InputPlane { .. })
        | Err(Error::InputPlaneSizeMismatch { .. })
        | Err(Error::InputChromaGeometry { .. }) => {}
        Err(err) => panic!("unexpected encoder frame-input error: {err:?}"),
    }
});

fn exercise_fixed_cases() {
    exercise_valid_frame(3, 5);

    let decoded = decoded_yuv420_frame(3, 5);
    let shared = SharedFrame::new(decoded);
    let retained = RetainedFrame::from_shared_frame(
        FrameInfo::yuv420_8bit(FrameId::new(9), size(3, 5)),
        shared,
    )
    .unwrap_or_else(|err| panic!("valid retained frame should be accepted: {err:?}"));
    let retained_again = retained.share();
    assert_eq!(retained.handle_count(), 2);
    assert_eq!(retained_again.handle_count(), 2);
    assert_eq!(
        retained
            .as_frame()
            .unwrap_or_else(|err| panic!("retained frame borrow should be valid: {err:?}"))
            .y()
            .samples()
            .as_ptr(),
        retained_again
            .as_frame()
            .unwrap_or_else(|err| panic!("shared retained frame borrow should be valid: {err:?}"))
            .y()
            .samples()
            .as_ptr()
    );

    let mono = decoded_monochrome_frame(3, 5);
    let err = RetainedFrame::from_shared_frame(
        FrameInfo::yuv420_8bit(FrameId::new(10), size(3, 5)),
        SharedFrame::new(mono),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        Error::UnsupportedInputChromaSubsampling {
            chroma_subsampling: ChromaSubsampling::Monochrome
        }
    ));
}

fn assert_valid_frame(frame: Frame<'_>, width: usize, height: usize, y: &[u8], u: &[u8], v: &[u8]) {
    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.chroma_subsampling(), ChromaSubsampling::Yuv420);
    assert_eq!(frame.visible_luma_size(), size(width, height));
    assert_eq!(frame.y().samples().as_ptr(), y.as_ptr());
    assert_eq!(frame.u().samples().as_ptr(), u.as_ptr());
    assert_eq!(frame.v().samples().as_ptr(), v.as_ptr());
    assert_eq!(frame.y().visible_rows().len(), height);
}

fn exercise_valid_frame(width: usize, height: usize) {
    let luma_size = size(width, height);
    let chroma_size = PixelFormat::Yuv420
        .chroma_size(luma_size)
        .unwrap_or_else(|err| panic!("bounded chroma size should not overflow: {err:?}"))
        .unwrap_or_else(|| panic!("YUV420 must have chroma planes"));
    let y = vec![0_u8; width * height];
    let u = vec![0_u8; chroma_size.width() * chroma_size.height()];
    let v = vec![0_u8; chroma_size.width() * chroma_size.height()];

    let frame = Frame::from_planes(
        FrameInfo::yuv420_8bit(FrameId::new(0), luma_size),
        FramePlanesInput::yuv(
            FramePlaneInput::new(&y, width, rect(0, 0, width, height)),
            FramePlaneInput::new(
                &u,
                chroma_size.width(),
                rect(0, 0, chroma_size.width(), chroma_size.height()),
            ),
            FramePlaneInput::new(
                &v,
                chroma_size.width(),
                rect(0, 0, chroma_size.width(), chroma_size.height()),
            ),
        ),
    )
    .unwrap_or_else(|err| panic!("fixed valid frame should be accepted: {err:?}"));
    assert_eq!(frame.visible_luma_size(), size(width, height));
    assert_eq!(frame.u().visible_size(), chroma_size);
    assert_eq!(frame.v().visible_size(), chroma_size);
}

fn decoded_yuv420_frame(width: usize, height: usize) -> DecodedFrame<u8> {
    let luma_size = size(width, height);
    let chroma_size = PixelFormat::Yuv420
        .chroma_size(luma_size)
        .unwrap_or_else(|err| panic!("bounded chroma size should not overflow: {err:?}"))
        .unwrap_or_else(|| panic!("YUV420 must have chroma planes"));
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        ReconBitDepth::Eight,
        PixelFormat::Yuv420,
        luma_size,
        rect(0, 0, width, height),
    )
    .unwrap_or_else(|err| panic!("fixed frame info should be valid: {err:?}"));
    let y = Plane::from_vec(
        luma_size,
        width,
        rect(0, 0, width, height),
        vec![0_u8; width * height],
    )
    .unwrap_or_else(|err| panic!("fixed luma plane should be valid: {err:?}"));
    let u = Plane::from_vec(
        chroma_size,
        chroma_size.width(),
        rect(0, 0, chroma_size.width(), chroma_size.height()),
        vec![1_u8; chroma_size.width() * chroma_size.height()],
    )
    .unwrap_or_else(|err| panic!("fixed U plane should be valid: {err:?}"));
    let v = Plane::from_vec(
        chroma_size,
        chroma_size.width(),
        rect(0, 0, chroma_size.width(), chroma_size.height()),
        vec![2_u8; chroma_size.width() * chroma_size.height()],
    )
    .unwrap_or_else(|err| panic!("fixed V plane should be valid: {err:?}"));
    DecodedFrame::try_new(info, FramePlanes::new(y, Some(u), Some(v)))
        .unwrap_or_else(|err| panic!("fixed decoded frame should be valid: {err:?}"))
}

fn decoded_monochrome_frame(width: usize, height: usize) -> DecodedFrame<u8> {
    let luma_size = size(width, height);
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        ReconBitDepth::Eight,
        PixelFormat::Monochrome,
        luma_size,
        rect(0, 0, width, height),
    )
    .unwrap_or_else(|err| panic!("fixed monochrome info should be valid: {err:?}"));
    let y = Plane::from_vec(
        luma_size,
        width,
        rect(0, 0, width, height),
        vec![0_u8; width * height],
    )
    .unwrap_or_else(|err| panic!("fixed luma plane should be valid: {err:?}"));
    DecodedFrame::try_new(info, FramePlanes::new(y, None, None))
        .unwrap_or_else(|err| panic!("fixed monochrome frame should be valid: {err:?}"))
}

fn stride(width: usize, too_small: bool, input: &mut FuzzInput<'_>) -> usize {
    if too_small && width > 1 {
        width - 1
    } else {
        width + input.bounded_usize(MAX_STRIDE_PADDING)
    }
}

fn required_len(width: usize, height: usize, stride: usize) -> usize {
    if stride < width {
        stride.saturating_mul(height)
    } else {
        (height - 1) * stride + width
    }
}

fn maybe_truncated_len(required: usize, truncate: bool) -> usize {
    if truncate && required > 0 {
        required - 1
    } else {
        required
    }
}

fn size(width: usize, height: usize) -> PlaneSize {
    PlaneSize::new(width, height)
        .unwrap_or_else(|err| panic!("bounded nonzero size should be valid: {err:?}"))
}

fn rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
    PlaneRect::new(x, y, width, height)
        .unwrap_or_else(|err| panic!("bounded nonzero rect should be valid: {err:?}"))
}

struct FuzzInput<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> FuzzInput<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn byte(&mut self) -> u8 {
        let byte = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos = self.pos.saturating_add(1);
        byte
    }

    fn bounded_usize(&mut self, max_inclusive: usize) -> usize {
        usize::from(self.byte()) % (max_inclusive + 1)
    }
}
