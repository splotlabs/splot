# AV2 v1.0.0 — Annex B: Length delimited bitstream format

<!-- Verbatim mirror of the AOM AV2 v1.0.0 specification (© Alliance for Open Media). The PDF is normative; this is a faithful `pdftotext -layout` copy. See [./README.md](./README.md) and [./index.md](./index.md). Do not hand-edit: regenerate via scripts/spec/regenerate-av2-spec.sh. -->

<a id="s-annex-b"></a>

## Annex B: Length delimited bitstream format

```text
§   Annex B: Length delimited bitstream format
```

<a id="s-annex-b-1"></a>

### Annex B.1 Overview

```text
§   B.1.Overview
    § 5 Syntax structures define the syntax for OBUs. This annex defines a length-delimited format for
    packing OBUs into a bitstream.

    In derived specifications, such as container formats enabling storage of AV2 videos together with audio
    or subtitles, other methods of packing OBUs into a bitstream format are also allowed.

```

<a id="s-annex-b-2"></a>

### Annex B.2 Length delimited bitstream syntax

```text
§   B.2.Length delimited bitstream syntax

     bitstream( ) {
         while ( more_data_in_bitstream() ) {
             leb128() num_bytes_in_obu;
             open_bitstream_unit( num_bytes_in_obu )
         }
     }


```

<a id="s-annex-b-3"></a>

### Annex B.3 Length delimited bitstream semantics

```text
§   B.3.Length delimited bitstream semantics
    more_data_in_bitstream() is a system-dependent method of determining when the system reaches the
    end of the bitstream. The method returns 1 when there is more data to read, or 0 when at the end of the
    bitstream.

    num_bytes_in_obu specifies the length in bytes of the next OBU.

                                                                                    ↑ Back to Table of Contents




    AV2 Specification                                                                          Page 1112 of 1169
```
