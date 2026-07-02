## MODIFIED Requirements

### Requirement: First-inter-frame frontier interintra and per-unit intra subset
The decoder SHALL derive the block mode context after reference selection
per § 5.20.7.6, using the selected single reference or the compound
reference pair. For a WARPMV block signalling smooth-mask interintra it
SHALL predict the mapped intra mode per plane at the prediction-block size
(II_DC with the § 7.13.2.12 IBP DC modifier when enabled, II_V, II_H),
motion-compensate, and blend per § 7.13.3.30 with the § 7.13.3.29 mask,
deferring wedge interintra, II_SMOOTH, and the SIMPLE-path interintra tail
with structured diagnostics. For an intra-in-inter block with a
multi-transform-unit partition it SHALL predict each transform unit
per § 5.20.7.24 from just-reconstructed samples, marking `BlockDecoded`
per unit; partitioned modes whose per-unit inputs are not yet modelled
SHALL defer before any output.

#### Scenario: Perpendicular-split intra-in-inter block decodes bit-exact
- **GIVEN** `syn-2frame-txsplit-intra-inter-64x64-10bit-q100.ivf`
- **WHEN** the stream is decoded
- **THEN** both frames match the pinned avmdec-verified hashes

#### Scenario: SIMPLE-path interintra defers
- **GIVEN** `syn-3frame-simple-interintra-64x32-10bit.ivf`
- **WHEN** the stream is decoded
- **THEN** decode rejects with `inter_interintra_unimplemented` before
  any output
