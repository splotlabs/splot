## ADDED Requirements

### Requirement: First inter frame decode frontier support row
The decoder support model SHALL track `DECODE-FIRST-INTER-FRAME-FRONTIER` as a
distinct partial `splot-decode` row named `first-inter-frame-frontier`. The row
SHALL cite AV2 § 5.2.1, § 5.18.2, § 5.19, § 5.20, § 6.18, § 7.7, § 7.13.3.18, and
§ 7.23, SHALL record the honest planner-rejection test plus the conformance
manifest test, and SHALL carry the reciprocal LOCAL-REFERENCE-EVIDENCE pointer
for the two-frame inter fixture. The row SHALL keep the full inter decode slice
(the § 7.7 `get_ref_frames()` derivation, the § 5.18.2 inter frame-header shared
tail, the multi-frame planner/runtime, § 7.23 reference retention, § 5.20 inter
mode_info, § 7.11 MV derivation, § 7.13.3.18 motion compensation, frame output)
out of scope as deferred work, and SHALL NOT claim any inter frame decode.

#### Scenario: Matrix records narrow first-inter-frontier support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `first-inter-frame-frontier` appears with Feature ID
  `DECODE-FIRST-INTER-FRAME-FRONTIER`
- **AND** it is marked partial rather than supported for inter decode
- **AND** it does not claim inter frame-header parse, inter mode_info, motion
  compensation, or multi-frame output

#### Scenario: Inter decode conformance coverage is deferred
- **WHEN** decoder conformance coverage is generated
- **THEN** the tile group / payload syntax coverage remains partial for inter
  decode, because this frontier brick decodes no inter frame (it only commits the
  verified target fixture and pins the honest planner rejection)
- **AND** a dedicated `first-inter-frame-frontier` decoder conformance coverage
  row is deferred until the inter decode slice lands (§ 7.7 `get_ref_frames()` →
  the § 5.18.2 inter header tail → multi-frame planner/runtime → § 7.23 reference
  retention → § 5.20 inter mode_info → § 7.11 MV → § 7.13.3.18 motion
  compensation → frame output), at which point the row reflects real coverage
  rather than a planned target
