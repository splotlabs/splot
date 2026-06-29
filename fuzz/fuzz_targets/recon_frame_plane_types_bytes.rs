// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use core::mem;

use libfuzzer_sys::fuzz_target;
use splot_recon::{
    BitDepth, DecodedFrame, DecodedFrameInfo, FrameMut, FramePlanes, FrameRef, OutputIndex,
    PixelFormat, Plane, PlaneId, PlaneMut, PlaneRect, PlaneRef, PlaneSize, ReconError, ReconSample,
    SharedFrame,
};

const MAX_LUMA_WIDTH: usize = 16;
const MAX_LUMA_HEIGHT: usize = 16;
const MAX_CROP_ORIGIN: usize = 2;
const MAX_STORAGE_PADDING: usize = 2;
const MAX_STRIDE_PADDING: usize = 2;

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    exercise_idc_mapping(&mut input);
    exercise_fixed_invalid_cases();

    let selector = input.byte();
    let bit_depth = if selector & 0b0000_0001 == 0 {
        BitDepth::Eight
    } else {
        BitDepth::Ten
    };
    let pixel_format = match (selector >> 1) & 0b0000_0011 {
        0 => PixelFormat::Monochrome,
        1 => PixelFormat::Yuv420,
        2 => PixelFormat::Yuv422,
        _ => PixelFormat::Yuv444,
    };

    match (bit_depth, selector & 0b0000_1000 != 0) {
        (BitDepth::Eight, true) => run_case::<u16>(&mut input, bit_depth, pixel_format),
        (BitDepth::Eight, false) => run_case::<u8>(&mut input, bit_depth, pixel_format),
        (BitDepth::Ten, _) => run_case::<u16>(&mut input, bit_depth, pixel_format),
    }
});

fn run_case<T: ReconSample>(input: &mut FuzzInput<'_>, bit_depth: BitDepth, format: PixelFormat) {
    let Some(model) = FrameModel::new(input, bit_depth, format) else {
        return;
    };
    let Some(frame) = model.build_frame::<T>() else {
        return;
    };

    assert_frame_matches(&frame, &model);
    assert_frame_ref_matches(frame.as_frame_ref(), &model);

    let direct_ref = FrameRef::new(
        model.info,
        frame.y().as_plane_ref(),
        frame.u().map(Plane::as_plane_ref),
        frame.v().map(Plane::as_plane_ref),
    );
    assert!(direct_ref.is_ok());

    exercise_mutable_views::<T>();
    exercise_model_invalid_cases::<T>(&model);

    let shared = SharedFrame::new(frame);
    assert_eq!(shared.handle_count(), 1);
    assert_frame_ref_matches(shared.as_frame_ref(), &model);
    let shared_again = shared.share();
    assert_eq!(shared.handle_count(), 2);
    assert_eq!(shared_again.handle_count(), 2);
    assert_eq!(shared_again.get().info(), model.info);
    drop(shared_again);
    assert_eq!(shared.handle_count(), 1);
}

fn exercise_idc_mapping(input: &mut FuzzInput<'_>) {
    let bit_depth_idc = input.byte() % 4;
    match BitDepth::from_av2_bit_depth_idc(bit_depth_idc) {
        Ok(BitDepth::Ten) => assert_eq!(bit_depth_idc, 0),
        Ok(BitDepth::Eight) => assert_eq!(bit_depth_idc, 1),
        Err(ReconError::UnsupportedBitDepthIdc { idc }) => {
            assert_eq!(idc, bit_depth_idc);
            assert!(bit_depth_idc >= 2);
        }
        Err(err) => panic!("unexpected bit-depth idc error: {err:?}"),
    }

    let chroma_idc = input.byte() % 6;
    match PixelFormat::from_av2_chroma_format_idc(chroma_idc) {
        Ok(format) => {
            assert_eq!(format.chroma_format_idc(), chroma_idc);
            assert!(chroma_idc <= 3);
        }
        Err(ReconError::UnsupportedChromaFormatIdc { idc }) => {
            assert_eq!(idc, chroma_idc);
            assert!(chroma_idc > 3);
        }
        Err(err) => panic!("unexpected chroma idc error: {err:?}"),
    }
}

fn exercise_fixed_invalid_cases() {
    let storage = size(4, 4);
    let visible = rect(0, 0, 4, 4);

    assert_recon_error(
        Plane::from_vec(storage, 3, visible, vec![0_u8; 12]),
        |err| matches!(err, ReconError::StrideTooSmall { .. }),
    );
    assert_recon_error(
        Plane::from_vec(storage, 4, visible, vec![0_u8; 15]),
        |err| matches!(err, ReconError::BufferLengthMismatch { .. }),
    );
    assert_recon_error(
        DecodedFrameInfo::new(
            OutputIndex::new(0),
            BitDepth::Eight,
            PixelFormat::Yuv420,
            storage,
            rect(1, 0, 2, 2),
        ),
        |err| matches!(err, ReconError::CropOriginNotAligned { .. }),
    );

    let mono_info = frame_info(BitDepth::Eight, PixelFormat::Monochrome, storage, visible)
        .unwrap_or_else(|err| panic!("fixed monochrome info should be valid: {err:?}"));
    let y = plane_from_u16::<u8>(storage, 4, visible, &[0; 16])
        .unwrap_or_else(|| panic!("fixed luma plane should be valid"));
    let u = plane_from_u16::<u8>(size(2, 2), 2, rect(0, 0, 2, 2), &[0; 4])
        .unwrap_or_else(|| panic!("fixed chroma plane should be valid"));
    assert_recon_error(
        DecodedFrame::try_new(mono_info, FramePlanes::new(y, Some(u), None)),
        |err| matches!(err, ReconError::UnexpectedChromaPlane { plane: PlaneId::U }),
    );

    let yuv_info = frame_info(BitDepth::Eight, PixelFormat::Yuv420, storage, visible)
        .unwrap_or_else(|err| panic!("fixed yuv info should be valid: {err:?}"));
    let y = plane_from_u16::<u8>(storage, 4, visible, &[0; 16])
        .unwrap_or_else(|| panic!("fixed luma plane should be valid"));
    assert_recon_error(
        DecodedFrame::try_new(yuv_info, FramePlanes::new(y, None, None)),
        |err| matches!(err, ReconError::MissingChromaPlane { plane: PlaneId::U }),
    );

    let y = plane_from_u16::<u8>(storage, 4, visible, &[0; 16])
        .unwrap_or_else(|| panic!("fixed luma plane should be valid"));
    let u = plane_from_u16::<u8>(size(1, 2), 1, rect(0, 0, 1, 2), &[0; 2])
        .unwrap_or_else(|| panic!("fixed wrong-size chroma plane should be valid"));
    let v = plane_from_u16::<u8>(size(2, 2), 2, rect(0, 0, 2, 2), &[0; 4])
        .unwrap_or_else(|| panic!("fixed chroma plane should be valid"));
    assert_recon_error(
        DecodedFrame::try_new(yuv_info, FramePlanes::new(y, Some(u), Some(v))),
        |err| {
            matches!(
                err,
                ReconError::PlaneSizeMismatch {
                    plane: PlaneId::U,
                    ..
                }
            )
        },
    );

    let ten_bit_y = plane_from_u16::<u8>(storage, 4, visible, &[0; 16])
        .unwrap_or_else(|| panic!("fixed luma plane should be valid"));
    let ten_bit_info = frame_info(BitDepth::Ten, PixelFormat::Monochrome, storage, visible)
        .unwrap_or_else(|err| panic!("fixed ten-bit info should be valid: {err:?}"));
    assert_recon_error(
        DecodedFrame::try_new(ten_bit_info, FramePlanes::new(ten_bit_y, None, None)),
        |err| matches!(err, ReconError::SampleTypeUnsupportedBitDepth { .. }),
    );

    let out_of_range_y = plane_from_u16::<u16>(storage, 4, visible, &[256; 16])
        .unwrap_or_else(|| panic!("fixed out-of-range luma plane should construct"));
    assert_recon_error(
        DecodedFrame::try_new(mono_info, FramePlanes::new(out_of_range_y, None, None)),
        |err| {
            matches!(
                err,
                ReconError::SampleOutOfRange {
                    plane: PlaneId::Y,
                    ..
                }
            )
        },
    );

    let samples = [0_u8; 4];
    assert_recon_error(PlaneRef::new(&samples, 1, rect(0, 0, 2, 2)), |err| {
        matches!(err, ReconError::StrideTooSmall { .. })
    });

    let mut samples = [0_u8; 4];
    assert_recon_error(PlaneMut::new(&mut samples, 2, rect(0, 0, 2, 3)), |err| {
        matches!(err, ReconError::BufferLengthMismatch { .. })
    });
}

fn exercise_mutable_views<T: ReconSample>() {
    let info = frame_info(
        BitDepth::Eight,
        PixelFormat::Monochrome,
        size(4, 4),
        rect(0, 0, 4, 4),
    )
    .unwrap_or_else(|err| panic!("fixed mutable-view info should be valid: {err:?}"));
    let Some(mut samples) = samples_from_u16::<T>(&[0; 16]) else {
        return;
    };
    let y = PlaneMut::new(&mut samples, 4, rect(0, 0, 4, 4))
        .unwrap_or_else(|err| panic!("fixed mutable-view plane should be valid: {err:?}"));
    let mut frame = FrameMut::new(info, y, None, None)
        .unwrap_or_else(|err| panic!("fixed mutable frame should be valid: {err:?}"));
    assert_eq!(frame.info(), info);
    assert!(frame.u().is_none());
    assert!(frame.v().is_none());
    assert_eq!(frame.y().visible_size(), size(4, 4));
    assert_eq!(
        frame.plane(PlaneId::Y).map(PlaneMut::visible_size),
        Some(size(4, 4))
    );
    assert!(frame.plane_mut(PlaneId::U).is_none());

    let mut rows_seen = 0;
    for row in frame.y_mut().visible_rows_mut() {
        assert_eq!(row.len(), 4);
        rows_seen += 1;
    }
    assert_eq!(rows_seen, 4);

    let frame_ref = frame.as_frame_ref();
    assert_eq!(frame_ref.info(), info);
    assert_eq!(frame_ref.y().visible_size(), size(4, 4));
    assert!(frame_ref.u().is_none());
    assert!(frame_ref.v().is_none());
}

fn exercise_model_invalid_cases<T: ReconSample>(model: &FrameModel) {
    let invalid_stride = model.y.storage.width().saturating_sub(1);
    let invalid_stride_len = invalid_stride.saturating_mul(model.y.storage.height());
    let Some(invalid_stride_samples) = samples_from_u16::<T>(&vec![0; invalid_stride_len]) else {
        return;
    };
    assert_recon_error(
        Plane::from_vec(
            model.y.storage,
            invalid_stride,
            model.y.visible,
            invalid_stride_samples,
        ),
        |err| matches!(err, ReconError::StrideTooSmall { .. }),
    );

    let bad_len = model.y.required_samples().saturating_sub(1);
    let Some(bad_len_samples) = samples_from_u16::<T>(&vec![0; bad_len]) else {
        return;
    };
    assert_recon_error(
        Plane::from_vec(
            model.y.storage,
            model.y.stride,
            model.y.visible,
            bad_len_samples,
        ),
        |err| matches!(err, ReconError::BufferLengthMismatch { .. }),
    );
}

fn assert_frame_matches<T: ReconSample>(frame: &DecodedFrame<T>, model: &FrameModel) {
    assert_eq!(frame.info(), model.info);
    assert_eq!(frame.output_index(), model.info.output_index());
    assert_eq!(frame.bit_depth(), model.bit_depth);
    assert_eq!(frame.pixel_format(), model.pixel_format);
    assert_eq!(frame.coded_luma_size(), model.luma_storage);
    assert_eq!(frame.visible_luma_rect(), model.luma_visible);
    assert_plane_matches(frame.y(), &model.y);
    assert_eq!(
        frame.plane(PlaneId::Y).map(Plane::visible_size),
        Some(model.y.visible.size())
    );

    match (&model.u, frame.u()) {
        (Some(expected), Some(actual)) => assert_plane_matches(actual, expected),
        (None, None) => {}
        _ => panic!("U plane presence mismatch"),
    }
    match (&model.v, frame.v()) {
        (Some(expected), Some(actual)) => assert_plane_matches(actual, expected),
        (None, None) => {}
        _ => panic!("V plane presence mismatch"),
    }
}

fn assert_frame_ref_matches<T: ReconSample>(frame: FrameRef<'_, T>, model: &FrameModel) {
    assert_eq!(frame.info(), model.info);
    assert_plane_ref_matches(frame.y(), &model.y);
    assert_eq!(
        frame.plane(PlaneId::Y).map(|plane| plane.visible_size()),
        Some(model.y.visible.size())
    );

    match (&model.u, frame.u()) {
        (Some(expected), Some(actual)) => assert_plane_ref_matches(actual, expected),
        (None, None) => {}
        _ => panic!("U plane view presence mismatch"),
    }
    match (&model.v, frame.v()) {
        (Some(expected), Some(actual)) => assert_plane_ref_matches(actual, expected),
        (None, None) => {}
        _ => panic!("V plane view presence mismatch"),
    }
}

fn assert_plane_matches<T: ReconSample>(plane: &Plane<T>, model: &PlaneModel) {
    assert_eq!(plane.storage_size(), model.storage);
    assert_eq!(plane.stride_samples(), model.stride);
    assert_eq!(plane.visible_rect(), model.visible);
    assert_eq!(plane.visible_size(), model.visible.size());
    assert_eq!(plane.required_samples(), model.samples.len());
    assert_eq!(
        plane.allocation_bytes(),
        model.samples.len() * mem::size_of::<T>()
    );
    assert_samples_match(plane.samples(), &model.samples);
    assert_visible_rows_match(plane.visible_rows(), model);
    assert_plane_ref_matches(plane.as_plane_ref(), model);
}

fn assert_plane_ref_matches<T: ReconSample>(view: PlaneRef<'_, T>, model: &PlaneModel) {
    assert_eq!(view.stride_samples(), model.stride);
    assert_eq!(view.visible_rect(), model.visible);
    assert_eq!(view.visible_size(), model.visible.size());
    assert_samples_match(view.samples(), &model.samples);
    assert_visible_rows_match(view.visible_rows(), model);
}

fn assert_samples_match<T: ReconSample>(actual: &[T], expected: &[u16]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().copied().zip(expected.iter().copied()) {
        assert_eq!(actual.to_u16(), expected);
    }
}

fn assert_visible_rows_match<'a, T, I>(rows: I, model: &PlaneModel)
where
    T: ReconSample + 'a,
    I: IntoIterator<Item = &'a [T]>,
{
    let mut row_count = 0;
    for (row_index, row) in rows.into_iter().enumerate() {
        assert_eq!(row.len(), model.visible.width());
        for (column, sample) in row.iter().copied().enumerate() {
            assert_eq!(sample.to_u16(), model.visible_sample(row_index, column));
        }
        row_count += 1;
    }
    assert_eq!(row_count, model.visible.height());
}

struct FrameModel {
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
    luma_storage: PlaneSize,
    luma_visible: PlaneRect,
    info: DecodedFrameInfo,
    y: PlaneModel,
    u: Option<PlaneModel>,
    v: Option<PlaneModel>,
}

impl FrameModel {
    fn new(
        input: &mut FuzzInput<'_>,
        bit_depth: BitDepth,
        pixel_format: PixelFormat,
    ) -> Option<Self> {
        let visible_width = 1 + usize::from(input.byte()) % MAX_LUMA_WIDTH;
        let visible_height = 1 + usize::from(input.byte()) % MAX_LUMA_HEIGHT;
        let crop_x = aligned_crop(input.byte(), pixel_format.subsampling_x());
        let crop_y = aligned_crop(input.byte(), pixel_format.subsampling_y());
        let storage_pad_x = usize::from(input.byte()) % (MAX_STORAGE_PADDING + 1);
        let storage_pad_y = usize::from(input.byte()) % (MAX_STORAGE_PADDING + 1);
        let stride_padding = usize::from(input.byte()) % (MAX_STRIDE_PADDING + 1);

        let luma_storage = PlaneSize::new(
            crop_x + visible_width + storage_pad_x,
            crop_y + visible_height + storage_pad_y,
        )
        .ok()?;
        let luma_visible = PlaneRect::new(crop_x, crop_y, visible_width, visible_height).ok()?;
        let info = frame_info(bit_depth, pixel_format, luma_storage, luma_visible).ok()?;
        let y = PlaneModel::new(input, bit_depth, luma_storage, luma_visible, stride_padding)?;

        let (u, v) = match pixel_format.chroma_size(luma_visible.size()).ok()? {
            None => (None, None),
            Some(chroma_visible_size) => {
                let chroma_x = crop_x >> usize::from(pixel_format.subsampling_x());
                let chroma_y = crop_y >> usize::from(pixel_format.subsampling_y());
                let chroma_pad_x = usize::from(input.byte()) % (MAX_STORAGE_PADDING + 1);
                let chroma_pad_y = usize::from(input.byte()) % (MAX_STORAGE_PADDING + 1);
                let chroma_storage = PlaneSize::new(
                    chroma_x + chroma_visible_size.width() + chroma_pad_x,
                    chroma_y + chroma_visible_size.height() + chroma_pad_y,
                )
                .ok()?;
                let chroma_visible = PlaneRect::new(
                    chroma_x,
                    chroma_y,
                    chroma_visible_size.width(),
                    chroma_visible_size.height(),
                )
                .ok()?;
                let u = PlaneModel::new(
                    input,
                    bit_depth,
                    chroma_storage,
                    chroma_visible,
                    stride_padding,
                )?;
                let v = PlaneModel::new(
                    input,
                    bit_depth,
                    chroma_storage,
                    chroma_visible,
                    stride_padding,
                )?;
                (Some(u), Some(v))
            }
        };

        Some(Self {
            bit_depth,
            pixel_format,
            luma_storage,
            luma_visible,
            info,
            y,
            u,
            v,
        })
    }

    fn build_frame<T: ReconSample>(&self) -> Option<DecodedFrame<T>> {
        let y = self.y.build_plane::<T>()?;
        let u = match self.u.as_ref() {
            Some(plane) => Some(plane.build_plane::<T>()?),
            None => None,
        };
        let v = match self.v.as_ref() {
            Some(plane) => Some(plane.build_plane::<T>()?),
            None => None,
        };
        DecodedFrame::try_new(self.info, FramePlanes::new(y, u, v)).ok()
    }
}

struct PlaneModel {
    storage: PlaneSize,
    visible: PlaneRect,
    stride: usize,
    samples: Vec<u16>,
}

impl PlaneModel {
    fn new(
        input: &mut FuzzInput<'_>,
        bit_depth: BitDepth,
        storage: PlaneSize,
        visible: PlaneRect,
        stride_padding: usize,
    ) -> Option<Self> {
        let stride = storage.width().checked_add(stride_padding)?;
        let required = stride.checked_mul(storage.height())?;
        let mut samples = Vec::new();
        samples.try_reserve_exact(required).ok()?;
        for _ in 0..required {
            samples.push(input.sample(bit_depth));
        }

        Some(Self {
            storage,
            visible,
            stride,
            samples,
        })
    }

    fn required_samples(&self) -> usize {
        self.stride * self.storage.height()
    }

    fn build_plane<T: ReconSample>(&self) -> Option<Plane<T>> {
        let samples = samples_from_u16::<T>(&self.samples)?;
        Plane::from_vec(self.storage, self.stride, self.visible, samples).ok()
    }

    fn visible_sample(&self, visible_row: usize, column: usize) -> u16 {
        let row = self.visible.y() + visible_row;
        let index = row * self.stride + self.visible.x() + column;
        self.samples[index]
    }
}

struct FuzzInput<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> FuzzInput<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn byte(&mut self) -> u8 {
        let byte = self.data.get(self.offset).copied().unwrap_or(0);
        self.offset = self.offset.saturating_add(1);
        byte
    }

    fn word(&mut self) -> u16 {
        u16::from_le_bytes([self.byte(), self.byte()])
    }

    fn sample(&mut self, bit_depth: BitDepth) -> u16 {
        self.word() % (bit_depth.max_sample() + 1)
    }
}

fn aligned_crop(seed: u8, subsampling: u8) -> usize {
    let base = usize::from(seed) % (MAX_CROP_ORIGIN + 1);
    if subsampling == 0 {
        base
    } else {
        (base % 2) << usize::from(subsampling)
    }
}

fn plane_from_u16<T: ReconSample>(
    storage: PlaneSize,
    stride: usize,
    visible: PlaneRect,
    samples: &[u16],
) -> Option<Plane<T>> {
    Plane::from_vec(storage, stride, visible, samples_from_u16(samples)?).ok()
}

fn samples_from_u16<T: ReconSample>(samples: &[u16]) -> Option<Vec<T>> {
    let mut output = Vec::new();
    output.try_reserve_exact(samples.len()).ok()?;
    for sample in samples {
        output.push(T::try_from_u16(*sample).ok()?);
    }
    Some(output)
}

fn frame_info(
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
    storage: PlaneSize,
    visible: PlaneRect,
) -> Result<DecodedFrameInfo, ReconError> {
    DecodedFrameInfo::new(
        OutputIndex::new(
            u64::from(storage.width() as u32) << 32 | u64::from(storage.height() as u32),
        ),
        bit_depth,
        pixel_format,
        storage,
        visible,
    )
}

fn size(width: usize, height: usize) -> PlaneSize {
    PlaneSize::new(width, height)
        .unwrap_or_else(|err| panic!("fixed size should be valid: {err:?}"))
}

fn rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
    PlaneRect::new(x, y, width, height)
        .unwrap_or_else(|err| panic!("fixed rectangle should be valid: {err:?}"))
}

fn assert_recon_error<T, F>(result: Result<T, ReconError>, expected: F)
where
    F: FnOnce(&ReconError) -> bool,
{
    match result {
        Ok(_) => panic!("expected ReconError"),
        Err(err) => assert!(expected(&err), "unexpected ReconError: {err:?}"),
    }
}
