// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Explicit multistream atlas presentation (AV2 Annex D).

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufReader, Write};
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail, ensure};
use clap::Args;
use splot_core::headers::atlas_segment::AtlasModeInfo;
use splot_core::stream_reader::TemporalUnitReader;
use splot_decode::{
    AtlasCompositor, AtlasLayout, DataProps, DecodeLimitThreshold, DecodeLimits, Decoder,
    DecoderSettings, Picture, PictureSamples, PlaneId, SendStatus, ThreadCount,
};

const REORDER_WINDOW: usize = 32;

/// Arguments for optional atlas presentation.
#[derive(Args, Debug)]
pub(crate) struct ComposeArgs {
    /// Input AV2 stream in Annex B or IVF format.
    input: PathBuf,
    /// Write the raw composite planes to this file. Use '-' for standard output.
    #[arg(short, long)]
    output: PathBuf,
    /// Set the number of decoder threads.
    #[arg(long, default_value_t = ThreadCount::Auto)]
    threads: ThreadCount,
    /// Set separate byte limits for one decoded frame, all pending pictures, and the canvas with scratch.
    /// The compositor accepts pictures within a window of 32 input temporal units.
    #[arg(long, default_value = "268435456")]
    max_bytes: NonZeroU64,
}

pub(crate) fn run(args: &ComposeArgs) -> Result<ExitCode> {
    let input = File::open(&args.input)
        .with_context(|| format!("Cannot open {}.", args.input.display()))?;
    let mut units = TemporalUnitReader::new(BufReader::new(input));
    let byte_limit = DecodeLimitThreshold::Max(args.max_bytes.get());
    let limits = DecodeLimits::default().with_max_decoded_frame_bytes(byte_limit);
    let mut decoder = Decoder::new(
        DecoderSettings::default()
            .with_threads(args.threads)
            .with_limits(limits),
    )?;
    let mut output = CompositeOutput {
        compositor: AtlasCompositor::new(limits),
        pending: VecDeque::new(),
        retained_bytes: 0,
        max_bytes: args.max_bytes.get(),
        completed: [None; REORDER_WINDOW],
        latest_unit: 0,
    };
    super::decode::publish_composite(&args.output, |writer| {
        while let Some(unit) = units.next_temporal_unit()? {
            let index = u64::try_from(unit.index)?;
            // Extended layers in one temporal unit share an output time (AV2 § 7.3.1).
            let pts = i64::try_from(unit.pts.unwrap_or(index))
                .context("The presentation timestamp exceeds the signed 64-bit range.")?;
            let props = DataProps::default().with_pts(pts).with_user_data(index);
            while decoder.send_data(unit.bytes, props)? == SendStatus::Full {
                if let Some(picture) = decoder.get_picture()? {
                    output.push(picture, writer)?;
                }
            }
            if index == 0
                && units
                    .ivf_header()
                    .is_some_and(|header| header.fourcc != *b"AV02")
            {
                bail!("The IVF input must contain AV2.");
            }
        }
        if let Some(warning) = units.ivf_warning() {
            bail!("Invalid IVF input. {warning}");
        }
        decoder.drain();
        while let Some(picture) = decoder.get_picture()? {
            output.push(picture, writer)?;
        }
        if let Some(group) = output.pending.front() {
            bail!(
                "Temporal unit {} lacks one or more atlas pictures.",
                group.unit
            );
        }
        ensure!(
            output.completed.iter().any(Option::is_some),
            "The stream has no multistream atlas to compose."
        );
        Ok(())
    })?;
    Ok(ExitCode::SUCCESS)
}

struct PresentationGroup {
    unit: u64,
    pts: i64,
    layout: Arc<AtlasLayout>,
    required: u32,
    received: u32,
    bytes: u64,
    pictures: Vec<Picture>,
}

struct CompositeOutput {
    compositor: AtlasCompositor,
    pending: VecDeque<PresentationGroup>,
    retained_bytes: u64,
    max_bytes: u64,
    completed: [Option<u64>; REORDER_WINDOW],
    latest_unit: u64,
}

impl CompositeOutput {
    fn push(&mut self, picture: Picture, writer: &mut dyn Write) -> Result<()> {
        let Some(association) = picture
            .atlas_associations()
            .iter()
            .find(|a| a.scope.is_global())
        else {
            return Ok(());
        };
        let layout = association
            .layout
            .as_ref()
            .context("The referenced global atlas is unavailable.")?;
        let info = match &layout.atlas.mode_info {
            AtlasModeInfo::Multistream(info) | AtlasModeInfo::MultistreamAlpha(info) => info,
            _ => bail!("Only multistream atlas modes 3 and 4 support composition."),
        };
        let required = info
            .segments
            .iter()
            .fold(0u32, |mask, segment| mask | 1 << segment.input_stream_id);
        let layer = picture.extended_layer_id().get();
        let layer_bit = 1u32 << layer;
        if required & layer_bit == 0 {
            return Ok(());
        }
        let props = picture.props();
        let (unit, pts) = (props.user_data, props.pts);
        self.latest_unit = self.latest_unit.max(unit);
        let oldest = self.latest_unit.saturating_sub(REORDER_WINDOW as u64 - 1);
        ensure!(
            unit >= oldest && self.pending.iter().all(|group| group.unit >= oldest),
            "Pictures exceed the reorder window of {REORDER_WINDOW} temporal units."
        );
        ensure!(
            !self.completed.contains(&Some(unit)),
            "Temporal unit {unit} has an additional picture after composition."
        );
        let group = if let Some(index) = self.pending.iter().position(|group| group.unit == unit) {
            &mut self.pending[index]
        } else {
            self.pending.push_back_mut(PresentationGroup {
                unit,
                pts,
                layout: Arc::clone(layout),
                required,
                received: 0,
                bytes: 0,
                pictures: Vec::with_capacity(info.segments.len()),
            })
        };
        ensure!(
            Arc::ptr_eq(&group.layout, layout),
            "Temporal unit {unit} contains different global atlas layouts."
        );
        ensure!(
            group.received & layer_bit == 0,
            "Temporal unit {unit} contains multiple pictures for extended layer {layer}."
        );
        let bytes = [PlaneId::Y, PlaneId::U, PlaneId::V]
            .into_iter()
            .filter_map(|id| picture.plane(id))
            .map(|plane| match plane {
                PictureSamples::Eight(plane) => size_of_val(plane.samples()) as u64,
                PictureSamples::Ten(plane) => size_of_val(plane.samples()) as u64,
            })
            .sum::<u64>();
        ensure!(
            bytes <= self.max_bytes - self.retained_bytes,
            "Pending atlas pictures exceed the storage limit of {} bytes.",
            self.max_bytes
        );
        self.retained_bytes += bytes;
        group.bytes += bytes;
        group.received |= layer_bit;
        group.pictures.push(picture);
        while self
            .pending
            .front()
            .is_some_and(|group| group.required == group.received)
        {
            let Some(group) = self.pending.pop_front() else {
                break;
            };
            let pictures: Vec<_> = group.pictures.iter().collect();
            let canvas = self
                .compositor
                .compose(&group.layout, group.pts, &pictures)?;
            canvas.write_raw(writer)?;
            self.retained_bytes -= group.bytes;
            self.completed[(group.unit % REORDER_WINDOW as u64) as usize] = Some(group.unit);
        }
        Ok(())
    }
}
