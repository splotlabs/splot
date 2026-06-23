## Why

The current `splot decode /Users/bartosztomczyk/Documents/SplotLabs/ac0ej3.ivf`
gate is still a leading IVF-payload shape diagnostic. The first IVF record
contains `[OBU_TEMPORAL_DELIMITER, OBU_SEQUENCE_HEADER, OBU_CLOSED_LOOP_KEY,
OBU_REGULAR_TILE_GROUP]`, while the minimal runtime rejects any leading payload
that is not exactly `[TD, SEQ, CLK]` before it validates the active sequence.

That hides the first actual codec-support blocker in the mission stream:
`ac0ej3.ivf` is profile 0 10-bit 4:2:0, while the current minimal runtime only
supports 8-bit decoded samples.

## What Changes

- Keep requiring the first three leading OBUs to be `[TD, SEQ, CLK]`.
- Parse and validate the leading sequence header before rejecting any additional
  OBUs in that leading IVF payload.
- Reject otherwise supported streams that carry extra leading-payload OBUs after
  the key frame before any caller-visible output.
- Pin the leading payload ordering so the local mission regression parses the
  leading sequence before any additional leading-payload OBU rejection. This
  originally surfaced `unsupported_bit_depth`; the follow-on
  `decode-ac0ej3-10bit-sequence-frontier` and
  `decode-ac0ej3-sequence-chroma-frontier` changes move parsing to the key-frame
  header, and `decode-ac0ej3-wienerns-frontier` now reports the live gate as
  `unsupported_wienerns_filter`.

## Impact

- Touches only minimal runtime gate ordering, tests, and tracking docs.
- Does not add 10-bit decode, additional frame-unit decode in the leading IVF
  payload, prediction, filtering, residual, output, or conformance support.
- Does not make partial output visible on unsupported streams.
