## ADDED Requirements

### Requirement: PC-Wiener classification support row

The decoder support model SHALL track `RECON-PC-WIENER-CLASSIFICATION` as a
distinct `splot-recon` row named `pc-wiener-classification`. The row SHALL mark
only the AV2 §7.20.4 pixel-classified Wiener skip-filter classification math as
supported over caller-resolved source samples, caller-resolved `LrTxSkip` values,
active bit depth, and `base_q_idx`. It SHALL record that full §7.20 traversal,
§7.20.2 frame reads, runtime `FilterClass` grid storage, `SubclassLookup`
derivation, §7.20.3 filter invocation, runtime decode wiring, 10-bit output,
reference refresh, and successful ac0ej3 decode remain unsupported or partial
until separately proven.

#### Scenario: Matrix records narrow PC-Wiener progress

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `pc-wiener-classification` appears with Feature ID
  `RECON-PC-WIENER-CLASSIFICATION`
- **AND** it cites AV2 §7.20.4, AV2 §9.8, generated table drift checks, and
  focused `splot-recon` tests
- **AND** it does not claim runtime loop-restoration wiring, `FilterClass` grid
  retention, §7.20.3 filtering invocation, 10-bit output, reference refresh, or
  successful ac0ej3 decode
