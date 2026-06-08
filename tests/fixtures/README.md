# Test fixtures

Tiny AV2 Annex B length-delimited bitstreams used by the CLI integration tests
(`crates/splot-cli/tests/cli.rs`) and handy for manual `splot validate` / `splot
inspect`.

A stream is a sequence of `[LEB128 length][OBU]`. These are crafted by hand and
kept intentionally tiny; sequence-header fixtures include enough payload syntax
to pass the full §5.4 `sequence_header_obu()` parser.

| File | Bytes (hex) | What it is | `splot validate` |
|------|-------------|------------|------------------|
| `conformant.av2` | `01 08 0b 04 82 0c cf dc 00 01 00 04 00 02` | TemporalDelimiter (len 1, hdr `08`) + a complete single-picture SequenceHeader (len 11, hdr `04` + general §5.4.1 fields, all §5.4 child configs, `film_grain_params_present`, and `trailing_bits`) | conformant, exit `0` |
| `seq-header-tile-params.av2` | `01 08 0c 04 82 0c cf dc 00 01 00 04 00 14 80` | Same prefix but `seq_tile_info_present_flag = 1` with a uniform single-tile `tile_params()`, so `inspect --json` reports the sequence-header payload fully parsed (`AV2-5.4-SEQUENCE-HEADER`) | conformant, exit `0` |
| `bad-global-xlayer.av2` | `02 88 05` | TemporalDelimiter with an extension byte and `obu_xlayer_id = 5`; AV2 §6.2.2 requires `GLOBAL_XLAYER_ID` (31) | error, exit `1` |
| `truncated.av2` | `05 08` | Declares a 5-byte OBU but only the 1 header byte is present | parse error, exit `1` |
| `prefix-then-truncated.av2` | `01 08 05 08` | A valid TemporalDelimiter followed by a truncated OBU; `inspect` prints the valid prefix, then reports the tail error | parse error, exit `1` |
| `frame-header-prefix.av2` | `01 08 0b 04 82 0c cf dc 00 01 00 04 00 02 02 10 e0` | `conformant.av2` plus an `OBU_CLOSED_LOOP_KEY` (len 2, hdr `10`, payload `e0`) whose first tile group carries a frame header with `cur_mfh_id = 0` and `seq_header_id_in_frame_header = 0`; `inspect --json` reports a `frame_header_prefix` activation summary | conformant, exit `0` |
| `operating-point-set.av2` | `01 08 0c c8 1f 01 00 00 05 00 00 00 00 80 40` | TemporalDelimiter + a global `operating_point_set_obu` (len 12, hdr `c8 1f` = ext, `obu_type` 18, `obu_xlayer_id = 31`) with `ops_reset_flag = 0`, `ops_id = 0`, `ops_cnt = 1`, one minimal operating point (`ops_data_size = 5`, `ops_xlayer_map` selects layer 0, `ops_mlayer_info_idc = 0`), then the extensible OBU tail; `inspect --json` surfaces an `operating_point_set` view | conformant, exit `0` |
| `buffer-removal-timing.av2` | `01 08 0c c8 1f 01 00 00 05 00 00 00 00 80 40 04 bc 1f 81 40` | `operating-point-set.av2` plus an OPS-dependent `buffer_removal_timing_obu` (len 4, hdr `bc 1f` = ext, `obu_type` 15, `obu_xlayer_id = 31`) with `br_ops_dependent_flag = 1`, `br_ops_id = 0`, `br_ops_cnt = 1`; `br_ops_cnt` matches the active OPS `ops_cnt`, and `inspect --json` surfaces a `buffer_removal_timing` view | conformant, exit `0` |
| `quantizer-matrix.av2` | `01 08 0b 04 82 0c cf dc 00 01 00 04 00 02 04 58 00 02 c0` | TemporalDelimiter + the `conformant.av2` sequence header + a `quantizer_matrix_obu` (len 4, hdr `58` = `obu_type` 22) with `qm_bit_map = 1` (level 0), `qm_chroma_info_present_flag = 0` (1 plane), and `qm_is_default_flag = 1` (default matrix); `inspect --json` surfaces a `quantizer_matrix` view | conformant, exit `0` |
| `film-grain.av2` | `01 08 0b 04 82 0c cf dc 00 01 00 04 00 02 06 5c 01 80 00 00 40` | TemporalDelimiter + the `conformant.av2` sequence header + a `film_grain_obu` (len 6, hdr `5c` = `obu_type` 23) with `fgm_update_flags = 1` (slot 0), `fgm_chroma_idc = 0` (4:2:0), and one minimal `film_grain_model` (no scaling points, `ar_coeff_lag = 0`); `inspect --json` surfaces a `film_grain` view | conformant, exit `0` |
| `padding.av2` | `01 08 04 e4 1f ff 80` | TemporalDelimiter + a global `padding_obu` (len 4, hdr `e4 1f` = ext, `obu_type` 25, `obu_xlayer_id = 31`) with one arbitrary `obu_padding_byte` (`ff`) followed by a `trailing_bits()` byte (`80`); `inspect --json` surfaces a `padding` view (`padding_len = 1`, `trailing_len = 1`) | conformant, exit `0` |
| `metadata-short.av2` | `01 08 09 a0 1f 00 01 12 34 56 78 80` | TemporalDelimiter + a global `metadata_short_obu` (len 9, hdr `a0 1f` = ext, `obu_type` 8, `obu_xlayer_id = 31`) carrying `METADATA_TYPE_HDR_CLL` (header byte `00`, `metadata_type = 1`, a 4-byte `metadata_hdr_cll`, then the `trailing_bits()` byte `80`); `inspect --json` surfaces a `metadata_short` view | conformant, exit `0` |
| `metadata-group.av2` | `01 08 0e a4 1f 00 00 01 06 04 00 00 12 34 56 78 80` | TemporalDelimiter + a global `metadata_group_obu` (len 14, hdr `a4 1f` = ext, `obu_type` 9, `obu_xlayer_id = 31`) with `metadata_unit_cnt_minus_1 = 0` and one non-cancelled `METADATA_TYPE_HDR_CLL` unit (`metadata_type = 1`, `muh_header_size = 3`, `muh_payload_size = 4`, a 4-byte `metadata_hdr_cll`, then the `trailing_bits()` byte `80`); `inspect --json` surfaces a `metadata_group` view | conformant, exit `0` |

Header byte decoding (AV2 §5.2.2, MSB-first `f(1) f(5) f(2)`):

- `0x08` = `0_00010_00`: ext=0, `obu_type`=2 (TemporalDelimiter), `tlayer`=0. With no
  extension, the validator infers `xlayer = 31` for a TemporalDelimiter.
- `0x04` = `0_00001_00`: ext=0, `obu_type`=1 (SequenceHeader), `tlayer`=0.
- `0x88 0x05` = ext=1, `obu_type`=2, `tlayer`=0, then `mlayer`=0, `xlayer`=5.
- `0x10` = `0_00100_00`: ext=0, `obu_type`=4 (ClosedLoopKey), `tlayer`=0. Its payload
  byte `e0` = `111_00000` is the tile-group prefix `is_first_tile_group = 1`,
  `cur_mfh_id = uvlc(0) = 1`, `seq_header_id_in_frame_header = uvlc(0) = 1`.
- `0x58` = `0_10110_00`: ext=0, `obu_type`=22 (QuantizationMatrix), `tlayer`=0.
- `0x5c` = `0_10111_00`: ext=0, `obu_type`=23 (FilmGrain), `tlayer`=0.
- `0xe4 0x1f` = ext=1, `obu_type`=25 (Padding), `tlayer`=0, then `mlayer`=0, `xlayer`=31.
- `0xa0 0x1f` = ext=1, `obu_type`=8 (MetadataShort), `tlayer`=0, then `mlayer`=0, `xlayer`=31.
- `0xa4 0x1f` = ext=1, `obu_type`=9 (MetadataGroup), `tlayer`=0, then `mlayer`=0, `xlayer`=31.

Regenerate with `printf`, e.g.:

```bash
printf '\x01\x08\x0b\x04\x82\x0c\xcf\xdc\x00\x01\x00\x04\x00\x02' > conformant.av2
printf '\x01\x08\x0c\x04\x82\x0c\xcf\xdc\x00\x01\x00\x04\x00\x14\x80' > seq-header-tile-params.av2
printf '\x02\x88\x05'                                             > bad-global-xlayer.av2
printf '\x05\x08'                                                 > truncated.av2
printf '\x01\x08\x0b\x04\x82\x0c\xcf\xdc\x00\x01\x00\x04\x00\x02\x02\x10\xe0' > frame-header-prefix.av2
printf '\x01\x08\x0c\xc8\x1f\x01\x00\x00\x05\x00\x00\x00\x00\x80\x40' > operating-point-set.av2
printf '\x01\x08\x0c\xc8\x1f\x01\x00\x00\x05\x00\x00\x00\x00\x80\x40\x04\xbc\x1f\x81\x40' > buffer-removal-timing.av2
printf '\x01\x08\x0b\x04\x82\x0c\xcf\xdc\x00\x01\x00\x04\x00\x02\x04\x58\x00\x02\xc0' > quantizer-matrix.av2
printf '\x01\x08\x0b\x04\x82\x0c\xcf\xdc\x00\x01\x00\x04\x00\x02\x06\x5c\x01\x80\x00\x00\x40' > film-grain.av2
printf '\x01\x08\x04\xe4\x1f\xff\x80'                             > padding.av2
printf '\x01\x08\x09\xa0\x1f\x00\x01\x12\x34\x56\x78\x80'         > metadata-short.av2
printf '\x01\x08\x0e\xa4\x1f\x00\x00\x01\x06\x04\x00\x00\x12\x34\x56\x78\x80' > metadata-group.av2
```

The two sequence-header fixtures share the general §5.4.1 prefix `82 0c cf dc …`;
`conformant.av2` has `seq_tile_info_present_flag = 0` (last payload byte `02`), while
`seq-header-tile-params.av2` sets it to 1 with `allow_tile_info_change = 0` and a
uniform single-tile `tile_params()` (`uniform_tile_spacing_flag = 1`), ending the
payload with `14 80`.

These `.av2` files are deliberately tracked (the root `.gitignore` ignores `*.av2`
elsewhere but un-ignores this directory).
