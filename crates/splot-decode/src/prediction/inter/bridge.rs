// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Deterministic bridge-frame reconstruction (AV2 § 5.20.5.2).

use splot_core::headers::frame::FrameSize;
use splot_core::span::ByteOffset;
use splot_recon::{
    CurrentFrameWorkspace, DecodedFrame, DecodedFrameInfo, InterpolationFilter, OutputIndex,
    PlaneRect, PlaneSize, ReconSample,
};

use crate::error::Result;

use super::Mv;
use super::find_mv_stack::{TemporalMotionBlock, TemporalMotionField};
use super::mc::{InterBlockParams, McBlockRect, WorkspaceSink, motion_compensate_inter_block_into};
use super::reference::ReferenceSamples;

/// Reconstructs an `OBU_BRIDGE_FRAME` from its sole reference.
///
/// AV2 § 5.20.2.1 forces `bru_mode = BRU_INACTIVE`; § 5.20.5.2 then fixes every
/// block to reference 0, zero motion vector, skip, and `EIGHTTAP_SHARP`. With bridge
/// frame filters disabled by § 5.18.2, the resulting coded frame is a sharp-filtered,
/// zero-motion prediction from the selected reference, including reference scaling.
pub(crate) fn reconstruct<T: ReconSample>(
    reference: ReferenceSamples<'_, T>,
    frame_size: FrameSize,
    visible: PlaneRect,
    output_index: u64,
    offset: ByteOffset,
) -> Result<DecodedFrame<T>> {
    let width = usize::try_from(frame_size.width).map_err(|_| {
        splot_recon::ReconError::ArithmeticOverflow {
            context: "bridge frame width",
        }
    })?;
    let height = usize::try_from(frame_size.height).map_err(|_| {
        splot_recon::ReconError::ArithmeticOverflow {
            context: "bridge frame height",
        }
    })?;
    let luma_size = PlaneSize::new(width, height)?;
    let storage_width =
        width
            .checked_next_multiple_of(8)
            .ok_or(splot_recon::ReconError::ArithmeticOverflow {
                context: "bridge storage width",
            })?;
    let storage_height =
        height
            .checked_next_multiple_of(8)
            .ok_or(splot_recon::ReconError::ArithmeticOverflow {
                context: "bridge storage height",
            })?;
    visible.ensure_within(luma_size)?;
    let reference_info = reference.info();
    let info = DecodedFrameInfo::new(
        OutputIndex::new(output_index),
        reference_info.bit_depth(),
        reference_info.pixel_format(),
        luma_size,
        visible,
    )?
    .with_storage_luma_size(PlaneSize::new(storage_width, storage_height)?)?;
    let mut workspace = CurrentFrameWorkspace::new_recycled(info)?; // the § 7.23 copy below covers the whole frame rect
    motion_compensate_inter_block_into(
        &mut WorkspaceSink::Frame(&mut workspace),
        InterBlockParams::single(
            reference,
            McBlockRect::from_luma_rect(0, 0, storage_width, storage_height),
            Mv::ZERO,
            InterpolationFilter::EightTapSharp,
        )
        .with_chroma(!reference_info.pixel_format().is_monochrome()),
        offset,
    )?;
    Ok(workspace.freeze()?)
}

/// Builds the § 7.23 saved motion field for bridge blocks: every covered 8x8 cell
/// references the bridge source with a zero motion vector (§ 5.20.5.2).
pub(crate) fn motion_field(
    frame_size: FrameSize,
    current_order_hint: u32,
    reference_order_hint: u32,
) -> Option<TemporalMotionField> {
    let width = usize::try_from(frame_size.width).ok()?;
    let height = usize::try_from(frame_size.height).ok()?;
    let mi_cols = 2usize.checked_mul(width.checked_add(7)? >> 3)?;
    let mi_rows = 2usize.checked_mul(height.checked_add(7)? >> 3)?;
    let mut field = TemporalMotionField::new_with_metadata(
        mi_rows,
        mi_cols,
        true,
        (width, height),
        &[Some(reference_order_hint)],
    )?;
    field.record_block(TemporalMotionBlock::new(
        0,
        0,
        mi_cols,
        mi_rows,
        mi_rows,
        mi_cols,
        current_order_hint,
        [Some(reference_order_hint), None],
        [Mv::ZERO; 2],
        [None; 2],
    ));
    Some(field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use splot_recon::{BitDepth, FramePlanes, PixelFormat, Plane, PlaneId};

    #[test]
    fn bridge_reconstruction_preserves_mi_padding_for_later_references() -> Result<()> {
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            BitDepth::Eight,
            PixelFormat::Yuv420,
            PlaneSize::new(10, 14)?,
            PlaneRect::new(0, 0, 10, 14)?,
        )?
        .with_storage_luma_size(PlaneSize::new(16, 16)?)?;
        let mut source = CurrentFrameWorkspace::new(info, 0u8)?;
        for (plane, side) in [(PlaneId::Y, 16), (PlaneId::U, 8), (PlaneId::V, 8)] {
            let samples: Vec<u8> = (0..side * side).map(|i| (i % 251) as u8).collect();
            source.write_rect(plane, PlaneRect::new(0, 0, side, side)?, &samples, side)?;
        }
        let source = source.freeze()?;
        let bridge = reconstruct(
            ReferenceSamples::settled(&source),
            FrameSize::new(10, 14),
            info.visible_luma_rect(),
            1,
            ByteOffset::new(0),
        )?;
        assert_eq!(bridge.coded_luma_size(), info.coded_luma_size());
        assert_eq!(bridge.info().storage_luma_size(), info.storage_luma_size());
        assert_eq!(bridge.info().visible_luma_rect(), info.visible_luma_rect());
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            assert_eq!(bridge.plane(plane), source.plane(plane));
        }

        let from_source = reconstruct(
            ReferenceSamples::settled(&source),
            FrameSize::new(8, 8),
            PlaneRect::new(0, 0, 8, 8)?,
            2,
            ByteOffset::new(0),
        )?;
        let from_bridge = reconstruct(
            ReferenceSamples::settled(&bridge),
            FrameSize::new(8, 8),
            PlaneRect::new(0, 0, 8, 8)?,
            2,
            ByteOffset::new(0),
        )?;
        assert_eq!(from_bridge, from_source);
        Ok(())
    }

    #[test]
    fn bridge_reconstruction_scales_the_selected_reference() -> Result<()> {
        let source_size = PlaneSize::new(4, 4)?;
        let source_rect = PlaneRect::new(0, 0, 4, 4)?;
        let source_info = DecodedFrameInfo::new(
            OutputIndex::new(7),
            BitDepth::Eight,
            PixelFormat::Monochrome,
            source_size,
            source_rect,
        )?;
        let source = DecodedFrame::try_new(
            source_info,
            FramePlanes::new(
                Plane::from_vec(source_size, 4, source_rect, (0..16).collect::<Vec<u8>>())?,
                None,
                None,
            ),
        )?;

        let bridge = reconstruct(
            ReferenceSamples::settled(&source),
            FrameSize::new(2, 3),
            PlaneRect::new(0, 0, 2, 3)?,
            11,
            ByteOffset::new(0),
        )?;

        assert_eq!(bridge.output_index(), OutputIndex::new(11));
        assert_eq!(bridge.coded_luma_size(), PlaneSize::new(2, 3)?);
        let visible: Vec<_> = bridge.y().visible_rows().flatten().copied().collect();
        assert_ne!(visible, &[0, 1, 4, 5, 8, 9]);
        Ok(())
    }
}
