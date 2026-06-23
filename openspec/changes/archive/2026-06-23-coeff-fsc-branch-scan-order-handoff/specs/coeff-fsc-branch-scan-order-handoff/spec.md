## ADDED Requirements

### Requirement: FSC branch derives scan order from transform size
The decoder SHALL provide a crate-private loaded-but-unwired FSC/IDTX coefficient branch handoff for Feature ID `DECODE-COEFF-FSC-BRANCH-SCAN-ORDER` that derives `scan = get_scan(txSz, txClass)` from generated AV2 § 9.2 `Tx_Width[txSz]` / `Tx_Height[txSz]` values and decode-local § 8.3.2 `get_tx_class(PlaneTxType)` before delegating to the existing scan-extent FSC branch.

#### Scenario: Derived scan matches explicit scan branch
- **WHEN** a nonzero luma FSC branch is supplied a valid `txSz`, caller-resolved `PlaneTxType`, level config, and context geometry
- **THEN** the derived-scan handoff MUST produce the same branch result, tile CDF state, tile context state, consumed bits, and symbol count as the explicit scan-extent FSC branch using the corresponding § 5.20.7.30 scan table

#### Scenario: Invalid transform size is fail-atomic
- **WHEN** the FSC scan-order handoff receives a `txSz` outside the generated transform-size table domain
- **THEN** it MUST return a typed error before mutating tile CDF state, tile context state, consumed bits, or symbol count

#### Scenario: Invalid derived scan shape is fail-atomic
- **WHEN** the FSC scan-order handoff derives a scan shape outside the supported AV2 coefficient extents
- **THEN** it MUST return a typed error before mutating tile CDF state, tile context state, consumed bits, or symbol count

#### Scenario: All-zero FSC routing remains rejected without scan derivation
- **WHEN** the FSC scan-order handoff receives the all-zero branch arm
- **THEN** it MUST preserve the existing FSC branch all-zero rejection without requiring transform-size table lookup or scan derivation
