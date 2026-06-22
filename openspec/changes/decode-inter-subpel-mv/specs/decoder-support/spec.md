## ADDED Requirements

### Requirement: Sub-pel inter frame decode support row
The decoder support model SHALL track `DECODE-INTER-SUBPEL-MV` as a distinct
partial `splot-decode` row named `inter-subpel-mv`. The row SHALL cite AV2
§ 5.20.7.6, § 5.20.7.13, § 5.20.7.20, § 7.13.3.17, and § 7.13.3.18, SHALL record
the sub-pel decode tests plus the conformance manifest test, and SHALL carry the
reciprocal LOCAL-REFERENCE-EVIDENCE pointer for the two-frame sub-pel inter
fixture. The row SHALL document that the verified subset is the single-reference
NEWMV (or zero-MV NEARMV/GLOBALMV) skip=1 EighthPel block with a
SWITCHABLE-or-fixed interpolation filter, and SHALL keep inter residual (skip=0),
compound / multi-reference prediction, motion modes, non-64x64 / multi-block
inter, in-loop filters, and live AVM/dav2d invocation in CI out of scope as
deferred work.

#### Scenario: Matrix records narrow sub-pel inter support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `inter-subpel-mv` appears with Feature ID `DECODE-INTER-SUBPEL-MV`
- **AND** it is marked partial rather than supported for inter decode
- **AND** it does not claim inter residual, compound / multi-reference
  prediction, motion modes, or multi-block inter
