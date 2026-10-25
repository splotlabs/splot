// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Luma palette search: the most frequent values of a leaf, refined by one
//! k-means step, and the color map of a palette leaf.

use splot_decode::encoder_syntax::{LeafBlock, PALETTE_COLORS, Palette};

use crate::intra_frame::PaddedSource;
use crate::kernels::kernels;
use crate::tile_writer::{Result, TileWriterError};

/// Buffers reused across palette leaves.
#[derive(Default)]
pub(crate) struct PaletteSearch {
    block: Vec<u16>,
    sorted: Vec<u16>,
    runs: Vec<(u32, u16)>,
    map: Vec<u8>,
}

impl PaletteSearch {
    /// Reads the luma source of `leaf` and counts its values; returns the
    /// number of distinct values.
    pub(crate) fn count(&mut self, source: &PaddedSource, leaf: &LeafBlock) -> Result<usize> {
        self.read(source, leaf)?;
        self.sorted.clone_from(&self.block);
        self.sorted.sort_unstable();
        self.runs.clear();
        for &value in &self.sorted {
            match self.runs.last_mut() {
                Some((count, last)) if *last == value => *count += 1,
                _ => self.runs.push((1, value)),
            }
        }
        self.runs
            .sort_unstable_by_key(|&(count, value)| (core::cmp::Reverse(count), value));
        Ok(self.runs.len())
    }

    /// The palette of at most `size` colors of the leaf [`Self::count`]
    /// read: its `size` most frequent values, each moved to the mean of the
    /// samples nearest it unless it holds at least half of them; `None` for
    /// fewer than two colors.
    pub(crate) fn palette(&mut self, size: usize) -> Option<Palette> {
        let size = size.min(PALETTE_COLORS).min(self.runs.len());
        if size < 2 {
            return None;
        }
        let mut colors = [0u16; PALETTE_COLORS];
        for (color, &(_, value)) in colors.iter_mut().zip(&self.runs[..size]) {
            *color = value;
        }
        let colors_of = |colors: &mut [u16; PALETTE_COLORS]| colors[..size].sort_unstable();
        colors_of(&mut colors);
        if self.runs.len() == size {
            return Some(palette(colors, size));
        }
        self.map.resize(self.block.len(), 0);
        kernels().nearest_colours(&self.block, &colors[..size], &mut self.map);
        let mut sums = [(0u64, 0u64, 0u64); PALETTE_COLORS];
        for (&sample, &index) in self.block.iter().zip(&self.map) {
            let seed = colors[usize::from(index)];
            let sum = &mut sums[usize::from(index)];
            *sum = (
                sum.0 + u64::from(sample),
                sum.1 + 1,
                sum.2 + u64::from(sample == seed),
            );
        }
        for (color, (sum, count, exact)) in colors.iter_mut().zip(sums) {
            if exact * 2 < count {
                *color = ((sum + count / 2) / count) as u16;
            }
        }
        colors_of(&mut colors);
        let mut kept = 0;
        for index in 0..size {
            if kept == 0 || colors[index] != colors[kept - 1] {
                colors[kept] = colors[index];
                kept += 1;
            }
        }
        colors[kept..].fill(0);
        (kept >= 2).then(|| palette(colors, kept))
    }

    /// The color map of `palette` over the luma source of `leaf`: the
    /// nearest color of each sample.
    pub(crate) fn map(
        &mut self,
        source: &PaddedSource,
        leaf: &LeafBlock,
        palette: &Palette,
    ) -> Result<&[u8]> {
        self.read(source, leaf)?;
        self.map.resize(self.block.len(), 0);
        kernels().nearest_colours(&self.block, palette.colors(), &mut self.map);
        Ok(&self.map)
    }

    /// Copies the luma source of `leaf` into `self.block`.
    fn read(&mut self, source: &PaddedSource, leaf: &LeafBlock) -> Result<()> {
        let (w, h) = leaf.dimensions()?;
        let (x, y) = (leaf.col4() * 4, leaf.row4() * 4);
        let (luma, stride, ..) = source.luma();
        self.block.clear();
        for row in y..y + h {
            let samples = luma
                .get(row * stride + x..row * stride + x + w)
                .ok_or(TileWriterError::Shape("palette leaf outside the source"))?;
            self.block.extend_from_slice(samples);
        }
        Ok(())
    }
}

fn palette(colors: [u16; PALETTE_COLORS], size: usize) -> Palette {
    Palette {
        size: size as u8,
        colors,
    }
}
