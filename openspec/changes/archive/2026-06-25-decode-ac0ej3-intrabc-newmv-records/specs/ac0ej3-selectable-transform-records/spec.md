## MODIFIED Requirements

### Requirement: ac0ej3 selectable-transform IntrABC mode-info handoff

The decoder SHALL consume the observed AV2 §5.20.5.3 `use_intrabc = 1`
mode-info path and the bounded §5.20.5.4 IntrABC syntax in the local ac0ej3
Wiener NS LR selectable-transform path, retaining only facts that are parsed in
spec order and continuing to reject before unsupported prediction or output.

#### Scenario: active IntrABC NEWMV block-vector syntax is handed off

- **WHEN** the local ac0ej3 selectable-transform path reaches a supported
  luma/shared block with `use_intrabc = 1`
- **AND** §5.20.5.4 decodes `intrabc_mode = 0`
- **THEN** the runtime SHALL read the optional `intrabc_precision`, derive the
  bounded IntrABC reference block-vector candidates, consume §5.20.7.20
  `read_mv()` using `MV_INTRABC_CONTEXT` and the decoded `MvPrecision`, apply
  §5.20.7.13 `mv_clamp_to_integer`, and retain the resulting block vector.
- **AND** it SHALL stop before current-frame block-copy prediction with a
  structured unsupported diagnostic.

#### Scenario: active IntrABC NEARMV block-vector syntax is handed off

- **WHEN** the local ac0ej3 selectable-transform path reaches a supported
  luma/shared block with `use_intrabc = 1`
- **AND** §5.20.5.4 decodes `intrabc_mode = 1`
- **THEN** the runtime SHALL select the retained DRL candidate from the bounded
  IntrABC reference block-vector stack without reading `read_mv()`.
- **AND** it SHALL keep the handoff syntax-only, without claiming current-frame
  block-copy prediction or output.
