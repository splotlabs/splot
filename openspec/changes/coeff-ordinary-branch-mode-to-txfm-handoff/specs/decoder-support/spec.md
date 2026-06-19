## ADDED Requirements

### Requirement: Mode_To_Txfm ordinary branch support row

The decoder support model SHALL track
`DECODE-COEFF-ORDINARY-BRANCH-MODE-TO-TXFM-HANDOFF` as a distinct
loaded-but-unwired ordinary coefficient branch infrastructure row. The row SHALL
cite AV2 v1.0.0 §5.20.7.27, §5.20.7.29, and §9.2; SHALL name focused
ordinary-branch tests as proof; and SHALL keep full `compute_tx_type`,
`get_tx_set`, frame-state derivation of `enable_chroma_dctonly`, directional
wide-angle mapping, luma/inter/lossless branches, runtime `coeffs()`,
dequantization, reconstruction, output, reference refresh, and external decoder
invocation as residual work.

#### Scenario: Support matrix records subset handoff only

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** a `coeff-ordinary-branch-mode-to-txfm-handoff` row appears with
  Feature ID `DECODE-COEFF-ORDINARY-BRANCH-MODE-TO-TXFM-HANDOFF`
- **AND** broad coefficient-loop and runtime decode rows remain partial or
  unsupported until separately implemented
