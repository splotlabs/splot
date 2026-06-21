## ADDED Requirements

### Requirement: general low-frequency base-tier coefficient walk

The encoder coefficient tokenizer SHALL walk an arbitrary quantized 4x4 DCT_DCT luma
`Quant[16]` whose nonzero coefficients sit at scan indices 0..=1 (eob <= 2, the
§8.3.2 low-frequency region) with base-tier magnitudes 1..=4, and emit the ordered
AV2 §5.20.7.27 coefficient token stream the decoder coeff loop reads: a coded
`all_zero`, `eob_pt_16`, a reverse-scan base pass (`coeff_base_eob` at the
end-of-block coefficient and `coeff_base` for the rest, the latter with the §8.3.2
low-frequency luma context derived from the incrementally-built `Level[]`), a
reverse-scan interleaved sign pass (`dc_sign` CDF for the DC, `sign_bit` bypass for
every other coefficient), and chroma `all_zero`. A fully-zero block SHALL emit a
single `all_zero`. It SHALL reject a nonzero beyond scan index 1 and a magnitude
beyond 4 with typed errors. This is a private, non-emitting stage tracked by
`ENC-COEFF-GENERAL-WALK-LF-BASE`; it does not code coeff_br/golomb magnitudes,
eob > 2, high-frequency or chroma coefficients, emit syntax, or produce packets.

#### Scenario: an asymmetric eob=2 block emits the decoder-exact stream

- **WHEN** an asymmetric block (DC negative, AC positive, different magnitudes) is
  tokenized
- **THEN** the token stream is the reverse-scan base pass then sign pass in §5.20.7.27
  order, with the DC `coeff_base` context derived from the running `Level[]` and the
  AC `sign_bit` emitted before the DC `dc_sign`

#### Scenario: the emitted stream is internally reversible

- **WHEN** the token stream is roundtripped through the §8.2 coder and re-read by the
  recovery helper
- **THEN** the rebuilt signed `Quant[16]` equals the input over asymmetric values
- **AND** this is asserted as §8.2 self-consistency, not decoder/AVM conformance

#### Scenario: out-of-scope input is rejected

- **WHEN** a nonzero coefficient sits beyond scan index 1, or a magnitude exceeds 4
- **THEN** the tokenizer returns a typed unsupported-eob or unsupported-magnitude
  error without panicking
