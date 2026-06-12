# bitstream delta: frame-filtering-deblocking-gdf-cdef

Advances `AV2-5.18.5-FILTERING` and the § 5.18.7.9/.10 filter-param rows
on the intra frame-header path.

## ADDED Requirements

### Requirement: intra-path filter parameter parsing

The frame-header core parser SHALL parse `deblocking_filter_params()`
(§ 5.18.5.2, including the `cur_mfh_id > 0` arms consulting the resolved
multi-frame header's `mfh_deblocking_filter_update` /
`mfh_apply_deblocking_filter`), `gdf_params()` (§ 5.18.7.9), and
`cdef_params()` (§ 5.18.7.10) on the intra path, gated on the parsed
§ 5.4.10 sequence filter configuration, and SHALL advance its stop status
past them to the next unparsed structure. A frame whose referenced
multi-frame header is not resolvable in-band SHALL keep the existing
unsupported routing.

#### Scenario: intra frame parses filter params

- **WHEN** an intra frame header reaches the § 5.18.2 tail with a parsed
  sequence filter configuration
- **THEN** the deblocking, GDF, and CDEF parameters are parsed and the
  stop status names the next unparsed structure

#### Scenario: MFH deblocking arm

- **WHEN** a `cur_mfh_id > 0` frame's resolved MFH sets
  `mfh_deblocking_filter_update == 1`
- **THEN** the § 5.18.5.2 MFH arm is parsed per the mirror

#### Scenario: EOF inside filter params

- **WHEN** the payload ends inside any of the three structures
- **THEN** the parser reports the truncation without panicking
