# Test fixtures

Tiny AV2 Annex B length-delimited bitstreams used by the CLI integration tests
(`crates/splot-cli/tests/cli.rs`) and handy for manual `splot validate` / `splot
inspect`.

A stream is a sequence of `[LEB128 length][OBU]`. These are crafted by hand; only
the OBU **headers** matter to the current validator (payloads are opaque to it).

| File | Bytes (hex) | What it is | `splot validate` |
|------|-------------|------------|------------------|
| `conformant.av2` | `01 08 02 04 ab` | TemporalDelimiter (len 1, hdr `08`) + SequenceHeader (len 2, hdr `04` + 1 payload byte) | conformant, exit `0` |
| `bad-global-xlayer.av2` | `02 88 05` | TemporalDelimiter with an extension byte and `obu_xlayer_id = 5`; AV2 §6.2.2 requires `GLOBAL_XLAYER_ID` (31) | error, exit `1` |
| `truncated.av2` | `05 08` | Declares a 5-byte OBU but only the 1 header byte is present | parse error, exit `1` |
| `prefix-then-truncated.av2` | `01 08 05 08` | A valid TemporalDelimiter followed by a truncated OBU; `inspect` prints the valid prefix, then reports the tail error | parse error, exit `1` |

Header byte decoding (AV2 §5.2.2, MSB-first `f(1) f(5) f(2)`):

- `0x08` = `0_00010_00`: ext=0, `obu_type`=2 (TemporalDelimiter), `tlayer`=0. With no
  extension, the validator infers `xlayer = 31` for a TemporalDelimiter.
- `0x04` = `0_00001_00`: ext=0, `obu_type`=1 (SequenceHeader), `tlayer`=0.
- `0x88 0x05` = ext=1, `obu_type`=2, `tlayer`=0, then `mlayer`=0, `xlayer`=5.

Regenerate with `printf`, e.g.:

```bash
printf '\x01\x08\x02\x04\xAB' > conformant.av2
printf '\x02\x88\x05'         > bad-global-xlayer.av2
printf '\x05\x08'             > truncated.av2
```

These `.av2` files are deliberately tracked (the root `.gitignore` ignores `*.av2`
elsewhere but un-ignores this directory).
