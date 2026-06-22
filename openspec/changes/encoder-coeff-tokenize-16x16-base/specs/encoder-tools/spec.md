## ADDED Requirements

### Requirement: size-generic 16x16 base-pass coefficient tokenization

The encoder SHALL tokenize an arbitrary 16×16 DCT_DCT luma block in the §5.20.7.27 base pass
for eob 1..=32 through a size-generic walk shared with the 4×4 path, mirroring the decoder's
§8.3.2 contexts at 16×16 geometry (the `coeff_base_eob` band breaks at `num_coeffs/8` and
`/4` = 32 and 64; the LF boundary `row+col<4`). The 4×4 path SHALL remain byte-identical. An
eob above 32 (eobPt `>= 7`, beyond this base-pass scope) SHALL be rejected with a typed
error; the `eob_pt_extra` refinement is required only for eob `>= 65` (`eob_pt_256 == 7`).
This is tracked by `ENC-COEFF-TOKENIZE-16X16-BASE`.

#### Scenario: 16x16 base-pass blocks roundtrip

- **WHEN** asymmetric 16×16 luma blocks (DC+AC, LF, HF, eob~32) are tokenized
- **THEN** each roundtrips through one §8.2 coder, decoding back to the exact symbol sequence

#### Scenario: the 4x4 path is unchanged

- **WHEN** the existing 4×4 tokenizer + frame tests run after the refactor
- **THEN** they pass unchanged (byte-identical 4×4 output)

#### Scenario: eob above 32 is rejected

- **WHEN** a 16×16 block with eob > 32 is tokenized
- **THEN** the encoder returns a typed unsupported-eob error
