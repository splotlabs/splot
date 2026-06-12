# bitstream delta: frame-header-inter-reference-paths

Advances `AV2-5.18.2-FRAME-HEADER-INFO` (the non-intra control region)
and its child structures.

## ADDED Requirements

### Requirement: inter frame-header control-region parsing

The frame-header core parser SHALL parse the § 5.18.2 inter/TIP/bridge/
switch control region — primary-reference signaling, the inter refresh
branches, the explicit reference map, the BRU triple, ref-mvs/TMVP, the
TIP block, DRL and MV-precision fields, motion modes,
`read_interpolation_filter()` (§ 5.18.5.1), the with-refs and with-bridge
frame sizes (§ 5.18.4.2/.3), and the § 5.18.3 reference-distance
derivations — gated on the parsed sequence configuration and the modeled
reference state, converging into the shared tail. A branch whose
reference-state inputs are poisoned SHALL stop honestly with facts
preserved; locally decidable § 6 clauses on the new fields SHALL carry
their citations.

#### Scenario: inter header parses its control region

- **WHEN** an inter frame follows reference-state-grounded intra frames
- **THEN** its control region parses through the shared tail

#### Scenario: poisoned reference state stops honestly

- **WHEN** an inter branch needs slot facts the model has poisoned
- **THEN** the parse stops at that branch with earlier facts preserved

#### Scenario: invalid reference index is flagged

- **WHEN** a parsed `ref_frame_idx` references a slot the modeled state
  proves invalid
- **THEN** the diagnostic with its governing citation is emitted
