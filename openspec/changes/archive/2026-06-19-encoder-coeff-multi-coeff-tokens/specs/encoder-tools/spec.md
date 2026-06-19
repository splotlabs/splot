## ADDED Requirements

### Requirement: Encoder multi-coefficient token accessors

The encoder SHALL provide private multi-coefficient (eob > 1) token accessors
tracked by `ENC-COEFF-MULTI-TOKENS`: a coded luma `all_zero` token (symbol 0), a
parameterized `eob_pt_16` token (`eob_pt_16_token(coeff_cdf_q_ctx, eob_ctx,
symbol)`, where symbol 1 selects eob = 2), and a parameterized low-frequency
`coeff_base_eob` token (`coeff_base_lf_eob_token(coeff_cdf_q_ctx, ctx, level)`, with
symbol = level − 1). The accessors SHALL roundtrip through one in-tree AV2 § 8.2
symbol encoder/decoder via the generic coefficient-token CDF-row router, which SHALL
route the eob = 2 AC `coeff_base_eob` context (1). They SHALL NOT compose a
multi-coefficient trace, derive chroma / high-frequency contexts, emit `coeff_br`,
or produce a coded packet.

#### Scenario: Accessors carry the expected symbols

- **WHEN** the multi-coefficient token accessors are built for the minimal eob = 2
  block
- **THEN** the coded `all_zero` SHALL carry symbol 0, the `eob_pt_16` SHALL carry
  symbol 1 (eob = 2), and the low-frequency `coeff_base_eob` for base level 1 SHALL
  carry symbol 0 at context 1.

#### Scenario: The eob = 2 CDF subsequence roundtrips

- **WHEN** the eob = 2 CDF token subsequence (coded `all_zero`, `eob_pt_16` = 1, the
  AC `coeff_base_eob` at context 1, the DC `coeff_base` at context 1) is roundtripped
  through the generic router and one in-tree AV2 § 8.2 coder
- **THEN** the decoded symbols SHALL be `[0, 1, 0, 0]`.

#### Scenario: The accessors are not yet composed into a trace

- **WHEN** the multi-coefficient token accessors are available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a multi-coefficient trace or
  Baseline Encoder Profile v1 output from the accessors alone.
