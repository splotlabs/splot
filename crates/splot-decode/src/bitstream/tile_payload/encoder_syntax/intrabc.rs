// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Intra block copy of the encoder facade: the block vectors a leaf may
//! code (§ 6.17.3.3 `is_mv_valid`).

use core::ops::Range;

/// `INTRABC_DELAY_SB64` (§ 3).
const INTRABC_DELAY_SB64: i64 = 4;
/// `MI_SIZE` (§ 3).
const MI_SIZE: i64 = 4;

/// A leaf and the tile it lies in, in luma 4x4 units.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BvFrame {
    /// `MiRow`, `MiCol` of the leaf.
    pub(super) mi: (usize, usize),
    /// `Block_Width[MiSize]`, `Block_Height[MiSize]` in samples.
    pub(super) size: (usize, usize),
    /// `MiRowStart..MiRowEnd` and `MiColStart..MiColEnd`.
    pub(super) tile: (Range<usize>, Range<usize>),
    /// `Block_Width[SbSize]` (= `Block_Height[SbSize]`) in samples.
    pub(super) sb_size: usize,
}

/// Whether the whole-sample block vector `mv` (`[row, col]` in 1/8 samples)
/// passes § 6.17.3.3 `is_mv_valid` for a frame with `allow_local_intrabc =
/// 0` and `allow_global_intrabc = 1`, taking `sb64Residual = 0` (a subset of
/// the vectors `AllowExtraIBCRange` allows), for a leaf at least 8x8 whose
/// chroma is its own (so `ChromaMiRow/Col` equal `MiRow/Col`). The source
/// must also lie in the current or the previous superblock row, the global
/// area of the frame header semantics.
pub(super) fn global_bv_valid(frame: &BvFrame, mv: [i32; 2]) -> bool {
    if mv[0] & 7 != 0 || mv[1] & 7 != 0 || frame.size.0 < 8 || frame.size.1 < 8 {
        return false;
    }
    let (mi_row, mi_col) = (frame.mi.0 as i64, frame.mi.1 as i64);
    let (bw, bh) = (frame.size.0 as i64, frame.size.1 as i64);
    let (delta_row, delta_col) = (i64::from(mv[0] >> 3), i64::from(mv[1] >> 3));
    let src_top = mi_row * MI_SIZE + delta_row;
    let src_left = mi_col * MI_SIZE + delta_col;
    let src_bottom = src_top + bh;
    let src_right = src_left + bw;
    let (rows, cols) = &frame.tile;
    if src_top < rows.start as i64 * MI_SIZE
        || src_left < cols.start as i64 * MI_SIZE
        || src_bottom > rows.end as i64 * MI_SIZE
        || src_right > cols.end as i64 * MI_SIZE
    {
        return false;
    }
    let sb_h = frame.sb_size as i64;
    let active_sb_row = mi_row * MI_SIZE / sb_h;
    let active_sb64_col = (mi_col * MI_SIZE) >> 6;
    let src_sb_row = (src_bottom - 1) / sb_h;
    let src_sb64_col = (src_right - 1) >> 6;
    let total_sb64_per_row = ((cols.len() as i64 - 1) >> 4) + 1;
    let active_sb64 = active_sb_row * total_sb64_per_row + active_sb64_col;
    let src_sb64 = src_sb_row * total_sb64_per_row + src_sb64_col;
    if src_sb64 >= active_sb64 - INTRABC_DELAY_SB64 {
        return false;
    }
    let gradient = INTRABC_DELAY_SB64 + sb_h / 64;
    let wf_offset = gradient * (active_sb_row - src_sb_row);
    if src_sb_row > active_sb_row || src_sb64_col >= active_sb64_col - INTRABC_DELAY_SB64 + wf_offset
    {
        return false;
    }
    src_top / sb_h >= active_sb_row - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(mi: (usize, usize), size: (usize, usize)) -> BvFrame {
        BvFrame {
            mi,
            size,
            tile: (0..64, 0..160),
            sb_size: 64,
        }
    }

    /// § 6.17.3.3 for a 16x8 leaf at (64, 384) of a 640-wide tile of 64x64
    /// superblocks: in its own row the source must end four 64-wide columns
    /// before the current one; in the row above, before the wavefront.
    #[test]
    fn global_vectors_follow_the_superblock_delay() {
        let leaf = frame((16, 96), (16, 8));
        let sample = |dy: i32, dx: i32| [dy * 8, dx * 8];
        assert!(global_bv_valid(&leaf, sample(0, -272)));
        assert!(!global_bv_valid(&leaf, sample(0, -264)));
        assert!(global_bv_valid(&leaf, sample(-64, 0)));
        assert!(global_bv_valid(&leaf, sample(-64, 48)));
        assert!(!global_bv_valid(&leaf, sample(-64, 56)));
        assert!(!global_bv_valid(&leaf, sample(-128, 0)));
        assert!(!global_bv_valid(&leaf, sample(0, -400)));
        assert!(!global_bv_valid(&leaf, [4, -64 * 8 * 5]));
        assert!(!global_bv_valid(&frame((16, 96), (4, 8)), sample(-64, 0)));
    }

    #[test]
    fn vectors_leave_neither_the_tile_nor_the_frame_top() {
        let leaf = frame((0, 80), (8, 8));
        assert!(!global_bv_valid(&leaf, [-8, -64 * 8 * 5]));
        let mut tiled = frame((16, 96), (8, 8));
        tiled.tile.1 = 64..160;
        assert!(!global_bv_valid(&tiled, [0, -(96 - 60) * 4 * 8]));
        assert!(global_bv_valid(&tiled, [-64 * 8, -(96 - 64) * 4 * 8]));
    }
}
