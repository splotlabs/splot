// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Extended SDP regions (§ 5.20.3.1 `region_type`): an inter-frame node whose
//! `LUMA_PART` tree codes intra blocks only and whose chroma is one block.

use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::SymbolSink;

use super::super::cdf::TileCdfSelector;
use super::super::partition_allowed::PartitionTreeType;
use super::super::partition_traversal::{
    INTRA_REGION, MIXED_REGION, extended_sdp_chroma_call, intra_region_context,
    should_read_extended_sdp_region_type,
};
use super::{EncoderSyntaxError, IntraTileSyntax, Partition, PartitionNode, Result, rule};

impl IntraTileSyntax {
    /// Whether `node` codes `region_type` after `partition` (§ 5.20.3.1); the
    /// encoder codes no BRU, so every block is `BRU_ACTIVE`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid block size.
    pub fn codes_region_type(&self, node: PartitionNode, partition: Partition) -> Result<bool> {
        should_read_extended_sdp_region_type(&self.frame, node.0, partition, true)
            .map_err(rule("region_type"))
    }

    /// Writes the `region_type` of `node` after `partition`: `INTRA_REGION`
    /// when `intra`, else `MIXED_REGION`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] when `node` codes no
    /// `region_type`, and [`EncoderSyntaxError::Rule`] for a write failure.
    pub fn write_region_type(
        &mut self,
        node: PartitionNode,
        partition: Partition,
        intra: bool,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        if !self.codes_region_type(node, partition)? {
            return Err(EncoderSyntaxError::Unsupported(
                "region_type at a node that codes none",
            ));
        }
        let ctx = intra_region_context(node.0.b_size).map_err(rule("region_type"))?;
        let value = if intra { INTRA_REGION } else { MIXED_REGION };
        let row = self.tile_row(TileCdfSelector::RegionType { ctx })?;
        symbols
            .symbol(row, Symbol::new(value))
            .map_err(rule("region_type"))
    }

    /// The `LUMA_PART` children of the `INTRA_REGION` node `node` coded with
    /// `partition`, in syntax order, and the one `CHROMA_PART` block that
    /// follows them.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid partition of `node`.
    pub fn region_children(
        &self,
        node: PartitionNode,
        partition: Partition,
    ) -> Result<(Vec<PartitionNode>, PartitionNode)> {
        let call = node
            .0
            .with_tree_type(PartitionTreeType::LumaPart)
            .with_intra_region(true);
        let children = self.children(PartitionNode(call, node.1), partition)?;
        let chroma = extended_sdp_chroma_call(call);
        Ok((children, PartitionNode(chroma, chroma.own_chroma_ref())))
    }
}
