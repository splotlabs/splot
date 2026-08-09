// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Root partition token for the supported undivided general-intra block.

/// One `do_split` token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PartitionToken {
    symbol: u8,
}

impl PartitionToken {
    /// Returns this token's coded symbol value.
    pub(crate) const fn symbol(self) -> u8 {
        self.symbol
    }
}

/// The § 8.3.2 `PlaneStart` for the shared root partition tree.
pub(crate) const ROOT_PARTITION_PLANE_START: usize = 0;

/// The § 8.3.2 `do_split` context for the tile-origin 64x64 superblock.
pub(crate) const ROOT_64X64_DO_SPLIT_CTX: usize = 12;

/// Emits `do_split == false` (`PARTITION_NONE`) for the root 64x64 superblock.
pub(crate) const fn emit_root_do_split_none() -> PartitionToken {
    PartitionToken { symbol: 0 }
}
