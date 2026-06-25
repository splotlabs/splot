## ADDED Requirements

### Requirement: Current-frame IntrABC copy support row

The decoder support model SHALL record `RECON-INTRABC-CURRENT-FRAME-COPY` as a
distinct `splot-recon` source-backed row named
`recon-intrabc-current-frame-copy`. The row SHALL mark only checked
same-workspace current-frame rectangle copy as supported, SHALL cite AV2
§7.13.3.18 for the `refIdx == -1` current-frame source model, and SHALL keep
broad IntrABC runtime decode, decoded sample availability, residual
reconstruction, loop restoration, output/reference refresh, and AVM/dav2d
equality unclaimed.

#### Scenario: Support matrix lists IntrABC copy primitive

- **WHEN** decoder support status is regenerated
- **THEN** `recon-intrabc-current-frame-copy` appears with Feature ID
  `RECON-INTRABC-CURRENT-FRAME-COPY`
- **AND** it lists focused current-frame workspace copy tests
- **AND** it remains scoped to checked rectangle copy rather than broad ac0ej3
  decode support

## MODIFIED Requirements

### Requirement: ac0ej3 IntrABC transform-record support row

The decoder support model SHALL record the ac0ej3 IntrABC mode-info and
prediction-geometry handoff under
`DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS`. The support row SHALL describe
that the local ac0ej3 Wiener NS LR selectable-transform path consumes the
observed AV2 §5.20.5.3 `use_intrabc = 1` and bounded §5.20.5.4
`read_intrabc_info()` syntax into retained transform-record metadata, derives
checked tile-local luma current-frame prediction geometry, including the
BILINEAR source sample envelope and subpel phase, against the padded MI-domain
current-frame bounds from the retained §5.20.7.13 and §5.20.7.20 block vector,
and gates fallback block-vector use to cases where the tile-local prelude state
proves the §7.12.2 stack has no prior IntrABC spatial/ref-MV-bank candidates.
It SHALL continue to mark decoded sample population, loop-restoration
filtering/output, reference refresh, AVM/dav2d byte equality, and successful
ac0ej3 decode as unsupported or unclaimed.
When the local probe advances through the observed NEARMV/NEWMV block-vector
subcase, the row SHALL document the IntrABC prediction-geometry frontier while
keeping fabricated current-frame samples and output unsupported.

#### Scenario: Matrix evidence records the IntrABC prediction-geometry handoff

- **WHEN** decoder support status is validated after the IntrABC
  prediction-geometry handoff
- **THEN** `ac0ej3-selectable-transform-records` remains a partial row with
  Feature ID `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS`
- **AND** the row cites AV2 §5.20.5.3, §5.20.5.4, §5.20.6.1, §5.20.6.3,
  §5.20.7.13, §5.20.7.20, §5.20.7.27, §6.19.7.12, §7.12.2,
  §7.13.3.18, and §8.3.2 as applicable evidence sections
- **AND** it lists focused tests for active `use_intrabc` syntax, retained
  IntrABC defaults, block-vector prediction geometry, non-IntrABC regression
  behavior, and the local ac0ej3 runtime probe
- **AND** it records the next structured unsupported-feature reason reached by
  the local ac0ej3 probe

#### Scenario: IntrABC current-frame sample frontier is tracked

- **WHEN** the local ac0ej3 probe advances past the previous
  `unsupported_wienerns_lr_selectable_transform_records_intrabc_prediction`
  stop
- **THEN** `docs/DECODER-SUPPORT-MATRIX.toml` SHALL list the IntrABC
  prediction-geometry handoff, the focused tests, and the new
  post-prediction-geometry unsupported reason.
- **AND** it SHALL continue to state that decoded sample population, loop
  restoration, output, reference refresh, AVM/dav2d byte equality, and full
  ac0ej3 decode are not claimed.

#### Scenario: Broad IntrABC and reconstruction claims remain absent

- **WHEN** decoder support and feature status are regenerated
- **THEN** broad IntrABC runtime decode, populated decoded frame samples,
  loop-restoration filtering/output, reference refresh, AVM/dav2d byte
  equality, and successful ac0ej3 decode remain unclaimed until separately
  proven
