// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder partition-symbol emission for the minimal intra block-symbol trace.
//!
//! The AV2 § 5.20.4.1 `do_split` partition flag is the **first** symbol the decoder reads on
//! the general intra tile path (before any block mode or coefficient symbol). For the frozen
//! single-block tier the root 64x64 superblock is never split, so the encoder emits the
//! `do_split == false` (`PARTITION_NONE`) symbol. This module models that one token; the
//! `block_symbol_trace` module composes it with the mode and coefficient tokens.

// The encoder runtime does not yet consume these emitters (it returns `NeedMoreData`); they
// are exercised by the block-symbol-trace roundtrip tests, matching the sibling emission
// modules' policy.
#![allow(dead_code)]

/// AV2 § 5.20.4.1 partition syntax covered by the current encoder subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PartitionSyntax {
    /// `do_split` in AV2 § 5.20.4.1.
    DoSplit,
}

impl PartitionSyntax {
    const fn as_str(self) -> &'static str {
        match self {
            Self::DoSplit => "do_split",
        }
    }
}

/// Scoped default-CDF selector for one partition token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PartitionCdfRowSelector {
    /// `TileDoSplitCdf[plane_start][ctx]` (§ 8.3.2).
    DoSplit {
        /// The § 8.3.2 `PlaneStart` (0 for the shared, non-SDP partition tree).
        plane_start: usize,
        /// The § 8.3.2 `do_split` context.
        ctx: usize,
    },
}

impl PartitionCdfRowSelector {
    pub(crate) const fn syntax_name(self) -> &'static str {
        match self {
            Self::DoSplit { .. } => PartitionSyntax::DoSplit.as_str(),
        }
    }
}

/// One emitted partition token: its syntax, scoped CDF-row selector, and symbol value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PartitionToken {
    /// The AV2 partition syntax element.
    pub(crate) syntax: PartitionSyntax,
    /// The scoped default-CDF row this token is coded against.
    pub(crate) selector: PartitionCdfRowSelector,
    /// The symbol value coded for `syntax`.
    pub(crate) symbol: u8,
}

impl PartitionToken {
    /// Returns this token's scoped CDF-row selector.
    pub(crate) const fn selector(self) -> PartitionCdfRowSelector {
        self.selector
    }

    /// Returns this token's coded symbol value.
    pub(crate) const fn symbol(self) -> u8 {
        self.symbol
    }
}

/// The § 8.3.2 `PlaneStart` for the root partition tree: 0 (the shared luma/non-SDP tree;
/// the frozen tier requires `enable_sdp == false`).
pub(crate) const ROOT_PARTITION_PLANE_START: usize = 0;

/// The § 8.3.2 `do_split` context for the root 64x64 superblock:
/// `Partition_Size_Adjust[BLOCK_64X64] * 4 == 3 * 4 == 12` — both out-of-frame neighbours at
/// the tile origin are no smaller than the block, so the two neighbour context bits are 0
/// (`ctx = adj_size * 4 + ctx1 * 2 + ctx2`). Pinned empirically against the decoder's
/// `do_split_selector` while decoding the AVM-validated `syn-flat-intra-64x64-q80` fixture
/// (which read `DoSplit { plane_start: 0, ctx: 12 }`).
pub(crate) const ROOT_64X64_DO_SPLIT_CTX: usize = 12;

/// `do_split == false` (`PARTITION_NONE`): the root 64x64 superblock is one undivided block.
pub(crate) const DO_SPLIT_NONE_SYMBOL: u8 = 0;

/// Emits the AV2 § 5.20.4.1 `do_split == false` (`PARTITION_NONE`) token for the root 64x64
/// superblock — the first symbol the decoder reads on the general intra tile path.
pub(crate) const fn emit_root_do_split_none() -> PartitionToken {
    PartitionToken {
        syntax: PartitionSyntax::DoSplit,
        selector: PartitionCdfRowSelector::DoSplit {
            plane_start: ROOT_PARTITION_PLANE_START,
            ctx: ROOT_64X64_DO_SPLIT_CTX,
        },
        symbol: DO_SPLIT_NONE_SYMBOL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_root_do_split_none_is_partition_none_at_ctx_12() {
        let token = emit_root_do_split_none();
        assert_eq!(token.syntax, PartitionSyntax::DoSplit);
        assert_eq!(token.symbol(), 0);
        assert_eq!(
            token.selector(),
            PartitionCdfRowSelector::DoSplit {
                plane_start: 0,
                ctx: 12,
            }
        );
        assert_eq!(token.selector().syntax_name(), "do_split");
    }
}
