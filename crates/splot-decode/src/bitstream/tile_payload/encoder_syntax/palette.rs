// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Luma palette syntax and color maps of the encoder facade: § 5.20.8.1
//! `palette_mode_info` and § 5.20.8.4 `palette_tokens`.

use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::SymbolSink;

use super::super::cdf::TileCdfSelector;
use super::super::general_intra_block::{ceil_log2_u32, palette_y_mode_allowed};
use super::super::intra_joint_modes::{LumaPalette, PALETTE_MAX_SIZE};
use super::{EncoderSyntaxError, IntraMode, IntraModes, IntraTileSyntax, LeafBlock, Result, rule};
use crate::residual::pipeline::palette_color_index_context;

/// `PALETTE_COLORS` (§ 3): the most colors of a luma palette.
pub const PALETTE_COLORS: usize = PALETTE_MAX_SIZE;

/// The luma palette of a `DC_PRED` leaf (§ 5.20.8.1).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Palette {
    /// `PaletteSizeY`, `2..=PALETTE_COLORS`.
    pub size: u8,
    /// `palette_colors_y`: the first `size` entries, strictly ascending.
    pub colors: [u16; PALETTE_COLORS],
}

impl Palette {
    /// The colors of the palette.
    #[must_use]
    pub fn colors(&self) -> &[u16] {
        &self.colors[..usize::from(self.size).min(PALETTE_COLORS)]
    }

    /// The decoder's palette, for samples of at most `bits` bits.
    pub(super) fn luma(self, bits: u32) -> Result<LumaPalette> {
        let colors = self.colors();
        if !(2..=PALETTE_COLORS).contains(&usize::from(self.size))
            || colors.windows(2).any(|pair| pair[0] >= pair[1])
            || colors.iter().any(|&color| u32::from(color) >> bits != 0)
        {
            return Err(EncoderSyntaxError::Unsupported("palette colors"));
        }
        Ok(LumaPalette::from_size_symbol(
            Symbol::new(self.size - 2),
            self.colors,
        ))
    }
}

/// The palette and geometry of the planned palette leaf.
#[derive(Clone, Copy)]
pub(super) struct PlannedPalette {
    pub(super) palette: LumaPalette,
    /// Luma `(x, y, width, height)` of the leaf.
    pub(super) rect: (usize, usize, usize, usize),
    /// `(onscreenWidth, onscreenHeight)` of § 5.20.8.4.
    pub(super) onscreen: (usize, usize),
}

impl IntraTileSyntax {
    /// Whether `leaf` codes `has_palette_y` under a `DC_PRED` luma mode
    /// (§ 5.20.5.3, § 5.20.7.5).
    #[must_use]
    pub fn palette_allowed(&self, leaf: &LeafBlock) -> bool {
        let size = leaf.frontier.b_size;
        matches!(
            (size.num_4x4_wide(), size.num_4x4_high()),
            (Ok(n4w), Ok(n4h)) if palette_y_mode_allowed(
                self.tools.chroma,
                IntraMode::Dc,
                size.index(),
                n4w,
                n4h,
            )
        )
    }

    /// Sets the row-major color map, one index per luma sample of the leaf,
    /// of the next palette leaf [`Self::plan_leaf`] plans. Samples past the
    /// frame take the decoder's copies of the last coded row and column.
    pub fn set_palette_map(&mut self, map: &[u8]) {
        self.palette_map.clear();
        self.palette_map.extend_from_slice(map);
    }

    /// Plans the palette of `modes` for `leaf`: checks the map and copies its
    /// samples past the frame as the decoder does.
    pub(super) fn plan_palette(
        &mut self,
        leaf: &LeafBlock,
        modes: IntraModes,
    ) -> Result<Option<PlannedPalette>> {
        let Some(palette) = modes.palette else {
            return Ok(None);
        };
        if modes.y_mode != IntraMode::Dc || !self.palette_allowed(leaf) {
            return Err(EncoderSyntaxError::Unsupported("palette in this block"));
        }
        let palette = palette.luma(u32::from(self.tools.bit_depth.bits()))?;
        let (w, h) = leaf.dimensions()?;
        let (x, y) = (leaf.col4() * 4, leaf.row4() * 4);
        let (mi_rows, mi_cols) = self.mi_dimensions();
        let onscreen = (
            w.min((mi_cols * 4).saturating_sub(x)),
            h.min((mi_rows * 4).saturating_sub(y)),
        );
        let map = &mut self.palette_map;
        if onscreen.0 == 0
            || onscreen.1 == 0
            || map.len() != w * h
            || map
                .iter()
                .any(|&index| usize::from(index) >= palette.size())
        {
            return Err(EncoderSyntaxError::Unsupported("palette color map"));
        }
        let (cols, rows) = onscreen;
        for row in map.chunks_exact_mut(w).take(rows) {
            let last = row[cols - 1];
            row[cols..].fill(last);
        }
        let (coded, past) = map.split_at_mut(rows * w);
        for row in past.chunks_exact_mut(w) {
            row.copy_from_slice(&coded[(rows - 1) * w..]);
        }
        Ok(Some(PlannedPalette {
            palette,
            rect: (x, y, w, h),
            onscreen,
        }))
    }

    /// Writes § 5.20.8.1 `palette_mode_info` of `leaf` under `modes`.
    pub(super) fn write_palette_mode_info(
        &mut self,
        leaf: &LeafBlock,
        modes: IntraModes,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        if modes.y_mode != IntraMode::Dc || !self.palette_allowed(leaf) {
            return match modes.palette {
                Some(_) => Err(EncoderSyntaxError::Unsupported("palette in this block")),
                None => Ok(()),
            };
        }
        let selector = TileCdfSelector::PaletteYMode;
        self.put(symbols, selector, usize::from(modes.palette.is_some()))?;
        let Some(palette) = modes.palette else {
            return Ok(());
        };
        let bits = u32::from(self.tools.bit_depth.bits());
        palette.luma(bits)?;
        let colors = palette.colors();
        self.put(symbols, TileCdfSelector::PaletteYSize, colors.len() - 2)?;
        let (cache, cache_len) = self.palette_y.palette_cache(leaf.row4(), leaf.col4());
        let mut taken = [false; PALETTE_COLORS];
        let mut count = 0;
        for &cached in &cache[..cache_len] {
            if count == colors.len() {
                break;
            }
            let hit = colors
                .iter()
                .zip(&mut taken)
                .find(|(color, taken)| **color == cached && !**taken);
            literal(symbols, u32::from(hit.is_some()), 1)?;
            if let Some((_, taken)) = hit {
                *taken = true;
                count += 1;
            }
        }
        let mut rest = [0u32; PALETTE_COLORS];
        let mut len = 0;
        for (&color, _) in colors.iter().zip(taken).filter(|&(_, taken)| !taken) {
            rest[len] = color.into();
            len += 1;
        }
        let rest = &rest[..len];
        let Some(&first) = rest.first() else {
            return Ok(());
        };
        literal(symbols, first, bits)?;
        let Some(largest) = rest.windows(2).map(|pair| pair[1] - pair[0] - 1).max() else {
            return Ok(());
        };
        let min_bits = bits - 3;
        let extra = (u32::BITS - largest.leading_zeros()).saturating_sub(min_bits);
        literal(symbols, extra, 2)?;
        let mut delta_bits = min_bits + extra;
        for pair in rest.windows(2) {
            literal(symbols, pair[1] - pair[0] - 1, delta_bits)?;
            delta_bits = delta_bits.min(ceil_log2_u32((1 << bits) - pair[1] - 1));
        }
        Ok(())
    }

    /// Writes § 5.20.8.4 `palette_tokens` of the planned leaf: nothing for a
    /// leaf without a palette. The scan direction is the one whose lines
    /// more often repeat the previous line or one index.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] without a planned leaf or for a symbol
    /// write failure.
    pub fn write_palette_tokens(&mut self, symbols: &mut impl SymbolSink) -> Result<()> {
        let Some(planned) = self.planned_palette()? else {
            return Ok(());
        };
        let (.., w, h) = planned.rect;
        let size = planned.palette.size();
        let (cols, rows) = planned.onscreen;
        let map = core::mem::take(&mut self.palette_map);
        let lines = |transposed: bool| -> (usize, usize) {
            if transposed {
                (cols, rows)
            } else {
                (rows, cols)
            }
        };
        let at = |transposed: bool, i: usize, j: usize| -> u8 {
            if transposed {
                map[j * w + i]
            } else {
                map[i * w + j]
            }
        };
        let identity = |transposed: bool, i: usize| -> u8 {
            let inner = lines(transposed).1;
            if i > 0 && (0..inner).all(|j| at(transposed, i, j) == at(transposed, i - 1, j)) {
                2
            } else {
                u8::from(
                    inner > 1 && (1..inner).all(|j| at(transposed, i, j) == at(transposed, i, 0)),
                )
            }
        };
        let copies = |transposed: bool| {
            (0..lines(transposed).0)
                .filter(|&i| identity(transposed, i) != 0)
                .count()
        };
        let transposed = w < 64 && h < 64 && copies(true) > copies(false);
        let result = (|| {
            if w < 64 && h < 64 {
                literal(symbols, u32::from(transposed), 1)?;
            }
            let (outer, inner) = lines(transposed);
            let mut previous = 3;
            for i in 0..outer {
                let flag = identity(transposed, i);
                self.put(
                    symbols,
                    TileCdfSelector::IdentityRowY { ctx: previous },
                    flag.into(),
                )?;
                previous = usize::from(flag);
                for j in 0..inner {
                    if flag == 2 || (flag == 1 && j > 0) {
                        continue;
                    }
                    let (r, c) = if transposed { (j, i) } else { (i, j) };
                    let index = map[r * w + c];
                    if r == 0 && c == 0 {
                        write_uniform(symbols, index.into(), size)?;
                        continue;
                    }
                    let (ctx, order) = palette_color_index_context(&map, w, r, c);
                    let symbol = order[..size]
                        .iter()
                        .position(|&color| color == index)
                        .ok_or(EncoderSyntaxError::Unsupported("palette color index"))?;
                    let selector = TileCdfSelector::PaletteYColorIndex {
                        palette_size: size,
                        ctx,
                    };
                    self.put(symbols, selector, symbol)?;
                }
            }
            Ok(())
        })();
        self.palette_map = map;
        result
    }
}

/// The color map of luma block `(x, y, width, height)` of the `planned`
/// palette leaf, whose map is `map`, copied into `unit` when the block does
/// not cover the leaf; `None` for a leaf without a palette.
pub(super) fn unit_map<'a>(
    (map, unit): (&'a [u8], &'a mut Vec<u8>),
    planned: Option<&PlannedPalette>,
    (x, y, width, height): (usize, usize, usize, usize),
) -> Result<Option<&'a [u8]>> {
    let Some(planned) = planned else {
        return Ok(None);
    };
    let (x0, y0, w, h) = planned.rect;
    if (x, y, width, height) == planned.rect {
        return Ok(Some(map));
    }
    if x < x0 || y < y0 || x - x0 + width > w || y - y0 + height > h {
        return Err(EncoderSyntaxError::Unsupported("palette transform block"));
    }
    unit.clear();
    for row in map.chunks_exact(w).skip(y - y0).take(height) {
        unit.extend_from_slice(&row[x - x0..x - x0 + width]);
    }
    Ok(Some(unit))
}

/// § 4.11.13 `NS(n)` of `value`.
fn write_uniform(symbols: &mut impl SymbolSink, value: u32, n: usize) -> Result<()> {
    let n = n as u32;
    let bits = u32::BITS - n.leading_zeros();
    let m = (1 << bits) - n;
    if value < m {
        literal(symbols, value, bits - 1)
    } else {
        literal(symbols, (value + m) >> 1, bits - 1)?;
        literal(symbols, (value + m) & 1, 1)
    }
}

fn literal(symbols: &mut impl SymbolSink, value: u32, bits: u32) -> Result<()> {
    symbols.literal(value, bits).map_err(rule("symbol write"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A palette codes 2 to 8 strictly ascending colors that fit the bit
    /// depth; any other is refused, never coded.
    #[test]
    fn palettes_outside_the_syntax_are_refused() {
        let palette = |size: u8, colors: [u16; PALETTE_COLORS]| Palette { size, colors };
        let ascending = [3, 40, 41, 99, 128, 200, 230, 255];
        assert!(palette(8, ascending).luma(8).is_ok());
        assert!(palette(2, ascending).luma(8).is_ok());
        for size in [0, 1, 9, 255] {
            assert!(palette(size, ascending).luma(10).is_err(), "size {size}");
        }
        assert!(palette(3, [3, 3, 40, 0, 0, 0, 0, 0]).luma(8).is_err());
        assert!(palette(3, [40, 3, 99, 0, 0, 0, 0, 0]).luma(8).is_err());
        assert!(palette(2, [3, 256, 0, 0, 0, 0, 0, 0]).luma(8).is_err());
        assert!(palette(2, [3, 1023, 0, 0, 0, 0, 0, 0]).luma(10).is_ok());
    }
}
