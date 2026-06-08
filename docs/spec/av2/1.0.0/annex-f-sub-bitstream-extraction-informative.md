# AV2 v1.0.0 — Annex F: Sub-bitstream extraction (informative)

<!-- Verbatim mirror of the AOM AV2 v1.0.0 specification (© Alliance for Open Media). The PDF is normative; this is a faithful `pdftotext -layout` copy. See [./README.md](./README.md) and [./index.md](./index.md). Do not hand-edit: regenerate via scripts/spec/regenerate-av2-spec.sh. -->

<a id="s-annex-f"></a>

## Annex F: Sub-bitstream extraction (informative)

```text
§   Annex F: Sub-bitstream extraction (informative)
```

<a id="s-annex-f-1"></a>

### Annex F.1 General

```text
§   F.1.General
    This annex specifies processes for extracting sub-bitstreams from AV2 bitstreams based on operating
    point selection. The sub-bitstream extraction process allows decoders to selectively decode portions of a
    bitstream that match their capabilities or application requirements.

    An AV2 bitstream may contain one or more operating points, defined within OPS OBUs, that describe
    different combinations of extended layers, embedded layers, and temporal layers. A decoder can select an
    appropriate operating point and extract a sub-bitstream containing only the OBUs associated with that
    operating point.

    The extraction process differs depending on whether the bitstream is a multistream bitstream or a
    singlestream bitstream:

      • For multistream bitstreams, extraction involves selecting extended layers, embedded layers, and
        temporal layers from global operating point sets, and optionally refining individual extended layers
        using local operating point sets.
      • For singlestream bitstreams, extraction involves selecting embedded and temporal layers from local
        operating point sets.

    The processes defined in this annex are informative and represent one conformant approach to sub-
    bitstream extraction. Decoders may use alternative methods provided they produce equivalent results.

```

<a id="s-annex-f-2"></a>

### Annex F.2 Operating point usage

```text
§   F.2.Operating point usage
```

<a id="s-annex-f-2-1"></a>

#### Annex F.2.1 General decoder operation

```text
§   F.2.1.General decoder operation

    When decoding an AV2 bitstream, a decoder can select to decode the entire bitstream or can examine
    whether it contains operating points, defined within one or more OPS OBUs, which may be more
    appropriate given the decoder’s capabilities or the intended application.

    The decoder operation depends on whether the bitstream is a multistream bitstream or a singlestream
    bitstream.


      NOTE: The decoder modes described below allow selection of operating points that may retain a
      subset of the extended layers and embedded layers present in the bitstream. These processes are
      valuable for applications that require partial decoding of a bitstream. However, operating point
      selection does not change the conformance requirements defined in Annex A.2 Profiles. Without
      direction from application-level requirements external to this specification, a conformant decoder is
      expected to decode all extended layers and embedded layers present in the bitstream.

```

<a id="s-annex-f-2-2"></a>

#### Annex F.2.2 Multistream bitstream decoder operation

```text
§   F.2.2.Multistream bitstream decoder operation

    When decoding an AV2 bitstream, a decoder first invokes the operating point selection and analysis
    process (Annex F.3.1 Operating point selection and analysis process). This process examines the
    bitstream structure and determines whether it is a multistream or singlestream bitstream (see Step 2 of
    the process).



    AV2 Specification                                                                           Page 1139 of 1169
    If the process determines the bitstream is a multistream, the bitstream contains several extended layer
    sub-bitstreams. The bitstream may include an MSDO OBU and/or one or more LCR OBUs that describe
    the structure and properties of the bitstream and each associated extended layer sub-bitstream. The
    bitstream may also contain one or more global operating point sets providing operating points that span
    multiple extended layers.

    Each extended layer sub-bitstream has its own OBUs, including Sequence Header OBUs, MFH OBUs,
    video coding layer OBUs (CLK, OLK, TG, SEF, TIP, etc.), and other OBU types. Extended layers may also
    contain local operating point sets.

    For multistream bitstreams, a decoder may operate in one of the following modes (illustrated in the
    figure below):



                                               Multistream Bitstream Decoder Operation Modes


                                                                       Multistream Bitstream




                                                                              Decoder
                                                                           Operation Mode?




                    a) Full Bitstream                                  b) Per-Layer Operating                     c) Global Operating
                        Decoding                                           Point Selection                          Point Selection

                 Decode all extended layers                            Decode all extended layers,                   Select global OPS
                    based on MSDO/LCR                                   select local OPS per layer                 (extended/embedded/
                  and Sequence Headers                                 (refine embedded/temporal)                        temporal layers)




                                                                                                     a) Extended layers                b) Complete layer
                                                                                                            only                          specification
                                                                                                       Retain extended                      Full 3D extraction
                                                                                                         layers only                         (X/M/T layers)




          Legend:
          X = Extended layer, M = Embedded layer, T = Temporal layer



    Figure F.1: Multistream bitstream decoder operation modes showing the three decoding approaches: full
    bitstream decoding, per-layer operating point selection, and global operating point selection with its two
    sub-modes.


```

<a id="s-annex-f-2-2-1"></a>

##### Annex F.2.2.1 Full bitstream decoding

```text
§   F.2.2.1.Full bitstream decoding

    Decode the entire bitstream including all extended layers based on the information provided in the
    MSDO or global LCR OBUs, when present, and the associated Sequence Headers of each extended layer.


    AV2 Specification                                                                                                                                Page 1140 of 1169
```

<a id="s-annex-f-2-2-2"></a>

##### Annex F.2.2.2 Per-layer operating point selection

```text
§   F.2.2.2.Per-layer operating point selection

    Decode all extended layers associated with the bitstream, but for each extended layer examine if there
    are any local operating point sets that may be preferable for operation.

    The decoder invokes the operating point selection and analysis process defined in Annex F.3.1 Operating
    point selection and analysis process with input inputBitstream (the entire input bitstream). In this
    decoder mode, the abstract function global_operating_point_selection() returns an indication to decode
    all extended layers (no global operating point constraints), and the abstract function
    local_operating_point_selection(xLayerId) is called for each extended layer to potentially select a local
    operating point for embedded/temporal layer refinement.

    The process outputs the arrays OpRetentionMap, OpXLayerIsSelected, OpProfileIdc, OpLevelIdc,
    OpTierIdc, and OpMlayerCnt.

    The decoder then invokes the sub-bitstream extraction process defined in Annex F.3.2 Sub-bitstream
    extraction process with inputs: inputBitstream (the entire input bitstream) and OpRetentionMap. The
    process outputs subBitstream.

    The decoder then decodes subBitstream and uses the arrays OpProfileIdc, OpLevelIdc, OpTierIdc, and
    OpMlayerCnt for conformance verification of each independent extended layer that is still present in the
    subBitstream. Extended layers with OpXLayerIsSelected[xLayerId] == 0 are not selected and their
    corresponding entries in OpProfileIdc, OpLevelIdc, OpTierIdc, and OpMlayerCnt will have INVALID
    values.

```

<a id="s-annex-f-2-2-3"></a>

##### Annex F.2.2.3 Global operating point selection

```text
§   F.2.2.3.Global operating point selection

    Examine if one or more global operating point sets (obu_xlayer_id equal to GLOBAL_XLAYER_ID) are
    present. If yes, examine if there is a preferred operating point in one of these operating point sets based
    on application needs or device capabilities, and use its information to select which layers to decode.

    The decoder invokes the operating point selection and analysis process defined in Annex F.3.1 Operating
    point selection and analysis process with input inputBitstream (the entire input bitstream).

    A global operating point may specify extended layers only, or it may specify complete information about
    extended layers, embedded layers, and temporal layers. Depending on the level of detail provided in the
    selected operating point, the abstract function global_operating_point_selection() and the abstract
    function local_operating_point_selection(xLayerId) behave differently:

    a) Extended layers only

    If the operating point contains information about which extended layers to retain (via ops_xlayer_map),
    but does not provide complete details about their associated embedded and temporal layers (i.e.,
    ops_mlayer_map and ops_tlayer_map are not fully specified for all indicated extended layers), the decoder
    may choose between two approaches:

      • The abstract function global_operating_point_selection() returns the selected global operating point
        (globalOpsId, globalOpIdx), which determines the extended layers to retain. The abstract function
        local_operating_point_selection(xLayerId) returns an indication to decode all embedded and temporal
        layers for each selected extended layer (no further refinement). This results in an OpRetentionMap
        where only the selected extended layers have non-zero entries, and for each such extended layer, all
        embedded and temporal layer entries are set to 1.


    AV2 Specification                                                                            Page 1141 of 1169
      • Alternatively, the abstract function global_operating_point_selection() returns the selected global
        operating point (globalOpsId, globalOpIdx) to determine extended layers, and the abstract function
        local_operating_point_selection(xLayerId) examines local OPS information (if available) for each
        selected extended layer to refine embedded and temporal layers. This results in an OpRetentionMap
        with selective extended layers and refined embedded/temporal layers based on local operating points.

    The operating point selection and analysis process outputs the arrays OpRetentionMap,
    OpXLayerIsSelected, OpProfileIdc, OpLevelIdc, OpTierIdc, and OpMlayerCnt.

    The decoder then invokes the sub-bitstream extraction process defined in Annex F.3.2 Sub-bitstream
    extraction process with inputs: inputBitstream (the entire input bitstream) and OpRetentionMap. The
    process outputs subBitstream.

    The decoder then decodes subBitstream and uses the arrays OpProfileIdc, OpLevelIdc, OpTierIdc, and
    OpMlayerCnt for conformance verification of each independent extended layer that is still present in the
    subBitstream. Extended layers with OpXLayerIsSelected[xLayerId] == 0 are not selected and their
    corresponding entries in OpProfileIdc, OpLevelIdc, OpTierIdc, and OpMlayerCnt will have INVALID
    values.

    b) Complete layer specification

    If the operating point contains complete information about the extended layers (via ops_xlayer_map),
    embedded layers (via ops_mlayer_map), and temporal layers (via ops_tlayer_map) that should be
    retained, the abstract function global_operating_point_selection() returns the selected global operating
    point (globalOpsId, globalOpIdx), and the operating point selection and analysis process uses the
    complete layer information from the global OPS to build the OpRetentionMap (Step 4 may use global OPS
    embedded/temporal layer information instead of calling local_operating_point_selection).

    The operating point selection and analysis process outputs the arrays OpRetentionMap,
    OpXLayerIsSelected, OpProfileIdc, OpLevelIdc, OpTierIdc, and OpMlayerCnt.

    The decoder then invokes the sub-bitstream extraction process defined in Annex F.3.2 Sub-bitstream
    extraction process with inputs: inputBitstream (the entire input bitstream) and OpRetentionMap. The
    process outputs subBitstream.

    The decoder then decodes subBitstream and uses the arrays OpProfileIdc, OpLevelIdc, OpTierIdc, and
    OpMlayerCnt for conformance verification of each independent extended layer that is still present in the
    subBitstream. Extended layers with OpXLayerIsSelected[xLayerId] == 0 are not selected and their
    corresponding entries in OpProfileIdc, OpLevelIdc, OpTierIdc, and OpMlayerCnt will have INVALID
    values.

```

<a id="s-annex-f-2-3"></a>

#### Annex F.2.3 Singlestream bitstream decoder operation

```text
§   F.2.3.Singlestream bitstream decoder operation

    As described in Annex F.2.2 Multistream bitstream decoder operation, when decoding an AV2 bitstream,
    a decoder first invokes the operating point selection and analysis process (Annex F.3.1 Operating point
    selection and analysis process). This process examines the bitstream structure and determines whether it
    is a multistream or singlestream bitstream (see Step 2 of the process).

    If the process determines the bitstream is singlestream (only a single distinct extended layer identifier is
    present), the bitstream contains only a single extended layer sub-bitstream. It may contain global level
    (obu_xlayer_id equal to GLOBAL_XLAYER_ID) OBU types such as temporal delimiters. The bitstream



    AV2 Specification                                                                             Page 1142 of 1169
    includes Sequence Header OBUs, MFH OBUs, video coding layer OBUs (CLK, OLK, TG, SEF, TIP, etc.),
    and other OBU types. It may also contain local operating point sets.

    For singlestream bitstreams, a decoder may operate in one of the following modes:

```

<a id="s-annex-f-2-3-1"></a>

##### Annex F.2.3.1 Full bitstream decoding

```text
§   F.2.3.1.Full bitstream decoding

    Decode the entire bitstream based on its Sequence Header information. No extraction is performed. The
    output is identical to the input bitstream, and the profile, tier, and level information are as indicated in
    the sequence header.

```

<a id="s-annex-f-2-3-2"></a>

##### Annex F.2.3.2 Local operating point selection

```text
§   F.2.3.2.Local operating point selection

    Examine if local OPS information exists, and if so, select a local operating point based on the application
    and capabilities of the device. In this case, retain only the embedded and temporal layers in the bitstream
    that correspond to the selected local operating point, and discard the others.

    The decoder invokes the operating point selection and analysis process defined in Annex F.3.1 Operating
    point selection and analysis process with input inputBitstream (the entire input bitstream). In this
    decoder mode, since the bitstream is a singlestream bitstream (single extended layer), the abstract
    function global_operating_point_selection() returns an indication to decode all extended layers (which is
    effectively the single extended layer present), and the abstract function
    local_operating_point_selection(xLayerId) is called for the single extended layer to select a local
    operating point for embedded/temporal layer refinement.

    The process outputs OpRetentionMap (with non-zero entries only for the single extended layer),
    OpXLayerIsSelected (with only one entry set to 1), OpProfileIdc, OpLevelIdc, OpTierIdc, and
    OpMlayerCnt.

    The decoder then invokes the sub-bitstream extraction process defined in Annex F.3.2 Sub-bitstream
    extraction process with inputs: inputBitstream (the entire input bitstream) and OpRetentionMap. The
    process outputs subBitstream.

    The decoder then decodes subBitstream and uses the values OpProfileIdc[xLayerId],
    OpLevelIdc[xLayerId], OpTierIdc[xLayerId], and OpMlayerCnt[xLayerId] (where xLayerId is the single
    extended layer identifier, typically 0) for conformance verification of the extended layer that is still
    present in the subBitstream.

```

<a id="s-annex-f-3"></a>

### Annex F.3 Sub-bitstream extraction processes

```text
§   F.3.Sub-bitstream extraction processes
    The following examples illustrate the sub-bitstream extraction process for both multistream and
    singlestream scenarios, showing how OBUs are filtered based on the selected operating point.




    AV2 Specification                                                                             Page 1143 of 1169
                                                                                           Multistream Sub-bitstream Extraction Example

   Legend:
             TD                  Global LCR                     Global OPS                            SH                    Local LCR                       Local OPS                       Frame(xId:0)                     Frame(xId:1)      Dropped

   Frame format: [xId:mId:tId] where x=extended, m=embedded, t=temporal layer ID



  Input Multistream Bitstream:
                                                        LCR        LCR          SH           SH            OPS
   TU 1 (t=0):        TD      G-LCR       G-OPS          [0]        [1]         [0]          [1]            [0]   [0:0:0]   [0:1:0]     [1:0:0]   [1:1:0]
                                                                                                                                                                                                                     Key Points
   TU 2 (t=1):        TD      [0:0:1]    [0:1:1]     [1:0:1]
                                                                                                                                                                                Input Bitstream:
                                                                                                                                                                                   • 2 extended layers (xId: 0, 1)
   TU 3 (t=0):        TD      [0:0:0]    [0:1:0]     [1:0:0]
                                                                                                                                                                                   • 2 embedded layers per xId (mId: 0, 1)
                                                                                                                                                                                   • 2 temporal layers (tId: 0, 1)
                                                                                                                                                                                   • TU 1: Contains tId=0 frames
                                                                                                                                                                                   • TU 2: Contains tId=1 frames
                                                                                                                                                                                   • TU 3: Contains tId=0 frames
                                                                 Multistream Extraction
                                                               Global Operating Point Selected:                                                                                 OBU Ordering per TU:
                                                                                                                                                                                   1. Temporal Delimiter (TD)
                                                        ops_xlayer_map[opsId][opIdx] = 0x0001                                                                                      2. Global LCR (first TU only)
                                                    ops_mlayer_map[31][opsId][opIdx][0] = 0x01                                                                                     3. Global OPS (optional)
                                                   ops_tlayer_map[31][opsId][opIdx][0][0] = 0x03                                                                                   4. All Local LCRs (one per xId)
                                                                 (Retain: xId=0, mId=0, tId=0 or tId=1)                                                                            5. All Sequence Headers (one per xId)
                                                                                                                                                                                   6. For each extended layer:
                                                                                                                                                                                        - Local OPS (optional)
                                                                                                                                                                                        - Frames (all embedded layers)

                                                                                                                                                                                Output Result:
  Output Sub-bitstream:                                                                                                                                                            • Only xId=0 retained
                                                        LCR        SH          OPS                                                                                                 • Only mId=0 retained
   TU 1 (t=0):        TD      G-LCR       G-OPS          [0]       [0]          [0]         [0:0:0]
                                                                                                                                                                                   • Only tId=0,1 retained
                                                                                                                                                                                   • All TDs preserved
   TU 2 (t=1):        TD      [0:0:1]
                                                                                                                                                                                   • Global LCR/OPS preserved
                                                                                                                                                                                   • Only SH/LCR/OPS for xId=0
   TU 3 (t=0):        TD      [0:0:0]




                                                                   Note: All Local LCRs and all Sequence Headers are grouped together after Global OBUs, before per-layer Local OPS and frames.
                                                                                      Frames in same temporal unit from same extended/embedded layer have the same temporal layer ID.




Figure F.2: Multistream sub-bitstream extraction example showing three temporal units (TUs) with OBUs
from two extended layers. The input bitstream contains properly ordered OBUs (Temporal Delimiter,
Global LCR, Global OPS, followed by per-layer Local LCR, Local OPS, Sequence Header, and frames).
Frames within the same temporal unit from the same extended/embedded layer have the same temporal
layer ID. The extraction process retains only OBUs matching the selected operating point (xId=0, mId=0,
tId=0 or 1).




AV2 Specification                                                                                                                                                                                                                     Page 1144 of 1169
                                                                                      Singlestream Sub-bitstream Extraction Example

       Legend:

                 TD = Temporal Delimiter                            LCR = Layer Config Record                               OPS = Operating Point Set                                    SH = Sequence Header

                 Frame (kept)                                       Frame (dropped)                   Frame format: [mId:tId] where m=embedded, t=temporal layer (single xId=0)




      Input Singlestream Bitstream (single extended layer xId=0):

       TU 1 (t=0):       TD       LCR         OPS          SH         [0:0]           [1:0]   [2:0]
                                                                                                                                                                                               Key Points

                                                                                                                                                  Input Bitstream:
       TU 2 (t=1):       TD        [0:1]       [1:1]        [2:1]
                                                                                                                                                    • Single extended layer (xId=0 implicit)
                                                                                                                                                    • 3 embedded layers (mId: 0, 1, 2)
       TU 3 (t=2):       TD        [0:2]       [1:2]        [2:2]                                                                                   • 3 temporal layers (tId: 0, 1, 2)
                                                                                                                                                    • TU 1: Contains tId=0 frames
       TU 4 (t=0):       TD        [0:0]       [1:0]        [2:0]                                                                                   • TU 2: Contains tId=1 frames
                                                                                                                                                    • TU 3: Contains tId=2 frames
                                                                                                                                                    • TU 4: Contains tId=0 frames


                                                                                                                                                  OBU Ordering per TU:

                                           Singlestream Extraction                                                                                  1. Temporal Delimiter (TD)
                                                                                                                                                    2. Local LCR (first TU only)
                                           Local Operating Point Selected:
                                                                                                                                                    3. Local OPS (optional, first TU only)
                                                                                                                                                    4. Sequence Header (first TU only)
                                 ops_mlayer_map[0][opsId][opIdx][0] = 0x03
                                                                                                                                                    5. Frames (all embedded layers with
                                ops_tlayer_map[0][opsId][opIdx][0][0] = 0x03
                                                                                                                                                        same temporal layer)
                                ops_tlayer_map[0][opsId][opIdx][0][1] = 0x03
                                             (Retain: mId=0,1 and tId=0,1 for both)
                                                                                                                                                  Extraction Process:
                                                                                                                                                    • Uses local OPS (ops_id[0])
                                                                                                                                                    • Selects embedded layers 0 and 1
                                                                                                                                                    • Selects temporal layers 0 and 1
                                                                                                                                                    • All OBUs from single xId=0
      Output Sub-bitstream:
       TU 1 (t=0):       TD       LCR         OPS          SH         [0:0]           [1:0]
                                                                                                                                                  Output Result:
                                                                                                                                                    • TU 3 completely removed (tId=2)
                                                                                                                                                    • Only frames with mId=0 or mId=1
       TU 2 (t=1):       TD        [0:1]       [1:1]
                                                                                                                                                    • Only frames with tId=0 or tId=1
                                                                                                                                                    • All TDs preserved
       TU 4 (t=0):       TD        [0:0]       [1:0]                                                                                                • LCR, OPS, SH preserved (mId=0, tId=0)



                                                                              Note: Singlestream has single extended layer. Frames in same TU have same temporal layer ID.




    Figure F.3: Singlestream sub-bitstream extraction example showing four temporal units (TUs) with a
    single extended layer (xId=0 implicit). The input contains Local LCR, Local OPS, and Sequence Header in
    the first temporal unit, followed by frames. Each temporal unit contains frames with the same temporal
    layer ID. The extraction process retains only frames matching the selected embedded layers (mId=0, 1)
    and temporal layers (tId=0, 1), completely removing TU 3 which contains only tId=2 frames.



```

<a id="s-annex-f-3-1"></a>

#### Annex F.3.1 Operating point selection and analysis process

```text
§   F.3.1.Operating point selection and analysis process

    This process analyzes an AV2 bitstream to determine which extended layers, embedded layers, and
    temporal layers should be retained based on operating point selection. The process builds a 3D layer
    retention map and extracts profile/level/tier information for conformance verification.

    The operating point selection and analysis process has the following input:

      • inputBitstream: The bitstream to analyze

    The process produces the following outputs:

      • retentionMap[32][8][4]: Three-dimensional retention map indicating which layers to retain (1) or
        discard (0)
      • xLayerIsSelected[32]: Array indicating which extended layers are selected (1) or not selected (0)




    AV2 Specification                                                                                                                                                                                           Page 1145 of 1169
  • profileIdc[32]: Profile identifier for each extended layer
  • levelIdc[32]: Level identifier for each extended layer
  • tierIdc[32]: Tier identifier for each extended layer
  • mlayerCnt[32]: Maximum embedded layer count for each extended layer

The process for determining the retention map and profile information is as follows:

Step 1: Initialize outputs

Initialize retentionMap[xLayerId][mLayerId][tLayerId] with all values set to 0 for xLayerId from 0 to 31,
mLayerId from 0 to 7, and tLayerId from 0 to 3.

Initialize xLayerIsSelected[xLayerId] with all values set to 0 for xLayerId from 0 to 31.

Initialize profileIdc[xLayerId], levelIdc[xLayerId], tierIdc[xLayerId], and mlayerCnt[xLayerId] with all
values set to INVALID for xLayerId from 0 to 31, where INVALID is a sentinel value (such as -1 for signed
integer representations) that indicates the value has not been set.

Step 2: Determine bitstream type and extended layers

Examine the bitstream structure to determine whether it is a multistream or singlestream bitstream:

  • If the bitstream contains two or more distinct obu_xlayer_id values (excluding obu_xlayer_id equal to
    GLOBAL_XLAYER_ID), the bitstream is multistream. Determine which extended layers are present
    using one of the following:

       ◦ If MultiStreamDecoderMode is equal to 1, use the extended layer information from
         num_streams_minus_2 and sub_xlayer_id[i].
       ◦ Else if global LCR OBUs are present (obu_xlayer_id equal to GLOBAL_XLAYER_ID), use the
         extended layer information from lcr_xlayer_map in the lcr_global_info().
       ◦ Otherwise, scan the bitstream to identify the distinct extended layer identifiers present.
  • Otherwise, the bitstream is singlestream. Mark only the single extended layer identifier present in
    the bitstream as the extended layer to process. This is typically obu_xlayer_id = 0 for most
    singlestream bitstreams.


  NOTE: A multistream bitstream that contains an MSDO OBU or global LCR OBUs provides
  structural metadata that enables the extraction process to enumerate extended layers without
  scanning the entire bitstream. When neither is present, the extraction process requires scanning the
  bitstream to identify distinct extended layer identifiers.


Additionally, examine the bitstream to determine:

  • Whether global OPS OBUs are present (obu_xlayer_id equal to GLOBAL_XLAYER_ID)
  • For each extended layer, whether local OPS OBUs are present




AV2 Specification                                                                             Page 1146 of 1169
Set the global OBU retention status in retentionMap[GLOBAL_XLAYER_ID]:

  • For multistream bitstreams where global OBUs are present (MSDO OBU, global LCR OBUs, or global
    OPS OBUs), set retentionMap[GLOBAL_XLAYER_ID][0][0] = 1 to indicate that global OBUs
    (obu_xlayer_id equal to GLOBAL_XLAYER_ID) should be retained in the extracted sub-bitstream.
  • Otherwise, retentionMap[GLOBAL_XLAYER_ID][0][0] remains 0 (initialized value).

Step 3: Global operating point selection

If one or more global operating point sets are present in the bitstream (OPS OBUs with obu_xlayer_id
equal to GLOBAL_XLAYER_ID):

  • Call the abstract function global_operating_point_selection()

This function represents device-specific or application-specific logic that selects a preferred global
operating point based on decoder capabilities and requirements. The function returns either:

  • A selected operating point identified by globalOpsId and globalOpIdx, or
  • An indication to decode all extended layers without global operating point constraints

If a global operating point is selected:

  • Set xLayerMap = ops_xlayer_map[globalOpsId][globalOpIdx]
  • For each extended layer identifier i from 0 to 30:

       ◦ If bit i is set in xLayerMap (i.e., (xLayerMap & (1 << i)) != 0), set xLayerIsSelected[i] = 1
       ◦ If bit i is not set, xLayerIsSelected[i] remains 0

If no global operating point is selected:

  • If the bitstream is a multistream (as determined in Step 2), set xLayerIsSelected[i] = 1 only for the
    extended layer identifiers i identified in Step 2
  • If the bitstream is a singlestream (as determined in Step 2), set xLayerIsSelected[i] = 1 only for the
    single extended layer identifier i identified in Step 2

Step 4: Local operating point selection and retention map construction

For each extended layer identifier xLayerId where xLayerIsSelected[xLayerId] == 1:

  • Call the abstract function local_operating_point_selection(xLayerId)

This function represents device-specific or application-specific logic that determines whether to refine the
embedded and temporal layers for this extended layer using a local operating point set. The function
returns either:

  • A selected local operating point identified by localOpsId and localOpIdx, or
  • An indication to decode all embedded and temporal layers for this extended layer

If a local operating point is selected for xLayerId:

  • Set mLayerMap = ops_mlayer_map[xLayerId][localOpsId][localOpIdx][xLayerId]



AV2 Specification                                                                                 Page 1147 of 1169
  • For each embedded layer identifier j from 0 to 7:

       ◦ If bit j is set in mLayerMap (i.e., (mLayerMap & (1 << j)) != 0), then:

            ▪ Set tLayerMap = ops_tlayer_map[xLayerId][localOpsId][localOpIdx][xLayerId][j]
            ▪ For each temporal layer identifier k from 0 to 3:

                    ▪ If bit k is set in tLayerMap (i.e., (tLayerMap & (1 << k)) != 0), then:

                        ▪ Set retentionMap[xLayerId][j][k] = 1

If no local operating point is selected for xLayerId, set retentionMap[xLayerId][j][k] = 1 for all j from 0 to
7 and all k from 0 to 3 (retain all embedded and temporal layers encountered).

If a global operating point was selected in Step 3 and provides embedded/temporal layer information for
xLayerId (via ops_mlayer_map and ops_tlayer_map), this information may be used instead of or in
combination with local operating point information, based on decoder policy.

Step 5: Extract profile, level, tier, and embedded layer count

For each extended layer identifier xLayerId where xLayerIsSelected[xLayerId] == 1:

Determine the profile, level, tier, and maximum embedded layer count for this extended layer from the
selected operating point (global or local) or from the bitstream metadata:

  • If a global operating point was selected and provides operational parameters for xLayerId:

       ◦ Set profileIdc[xLayerId] = ops_seq_profile_idc[GLOBAL_XLAYER_ID][globalOpsId][globalOpIdx]
         [xLayerId]
       ◦ Set levelIdc[xLayerId] = ops_level_idx[GLOBAL_XLAYER_ID][globalOpsId][globalOpIdx]
         [xLayerId]
       ◦ Set tierIdc[xLayerId] = ops_tier_flag[GLOBAL_XLAYER_ID][globalOpsId][globalOpIdx][xLayerId]
       ◦ Set mlayerCnt[xLayerId] = ops_mlayer_count[GLOBAL_XLAYER_ID][globalOpsId][globalOpIdx]
         [xLayerId]
  • Else if a local operating point was selected for xLayerId:

       ◦ Set profileIdc[xLayerId] = ops_seq_profile_idc[xLayerId][localOpsId][localOpIdx][xLayerId]
       ◦ Set levelIdc[xLayerId] = ops_level_idx[xLayerId][localOpsId][localOpIdx][xLayerId]
       ◦ Set tierIdc[xLayerId] = ops_tier_flag[xLayerId][localOpsId][localOpIdx][xLayerId]
       ◦ Set mlayerCnt[xLayerId] = ops_mlayer_count[xLayerId][localOpsId][localOpIdx][xLayerId]
  • Otherwise, extract the profile, level, and tier from the LCR OBU or Sequence Header OBU associated
    with this extended layer:

       ◦ Set profileIdc[xLayerId] from lcr_seq_profile_idc (LCR OBU) or seq_profile_idc (Sequence Header
         OBU)
       ◦ Set levelIdc[xLayerId] from lcr_max_level_idx (LCR OBU) or seq_level_idx (Sequence Header
         OBU)
       ◦ Set tierIdc[xLayerId] from lcr_tier_flag (LCR OBU) or seq_tier (Sequence Header OBU)



AV2 Specification                                                                               Page 1148 of 1169
           ◦ Set mlayerCnt[xLayerId] from lcr_max_mlayer_count (LCR OBU) or seq_max_mlayer_cnt_minus_1
             + 1 (Sequence Header OBU)

    Step 6: Return outputs

    Return retentionMap, xLayerIsSelected, profileIdc, levelIdc, tierIdc, and mlayerCnt.

```

<a id="s-annex-f-3-2"></a>

#### Annex F.3.2 Sub-bitstream extraction process

```text
§   F.3.2.Sub-bitstream extraction process

    This process extracts a sub-bitstream from an AV2 bitstream by filtering OBUs based on a 3D layer
    retention map. The process is purely mechanical and does not make selection decisions.

    The sub-bitstream extraction process has the following inputs:

      • inputBitstream: The bitstream from which to extract a sub-bitstream
      • retentionMap[32][8][4]: Three-dimensional retention map indicating which layers to retain

    The process produces the following output:

      • subBitstream: The extracted sub-bitstream containing only OBUs from retained layers

    The process for deriving the output sub-bitstream is as follows:

    Step 1: Initialize output

    Set the sub-bitstream subBitstream to be initially identical to the input bitstream inputBitstream.

    Step 2: Filter OBUs based on retention map

    For each OBU in subBitstream with obu_xlayer_id equal to xId, obu_mlayer_id equal to mId, and
    obu_tlayer_id equal to tId:

      • If the OBU type is OBU_TEMPORAL_DELIMITER:

           ◦ Retain the OBU in subBitstream (temporal delimiters are always retained regardless of layer
             selection)
      • Otherwise, determine if extended layer xId is selected by checking if there exists at least one pair (j,
        k) where retentionMap[xId][j][k] == 1. Set isXLayerSelected to true if such a pair exists, false
        otherwise.
      • If isXLayerSelected is false:

           ◦ Remove the OBU from subBitstream
      • Else if retentionMap[xId][mId][tId] == 0:

           ◦ If mId == 0 and tId == 0:

                ▪ Remove the OBU from subBitstream, except if the OBU type is OBU_SEQUENCE_HEADER,
                  OBU_LAYER_CONFIGURATION_RECORD, OBU_ATLAS_SEGMENT, or
                  OBU_OPERATING_POINT_SET
           ◦ Otherwise:

                ▪ Remove the OBU from subBitstream



    AV2 Specification                                                                             Page 1149 of 1169
    Step 3: Return output

    Return subBitstream.

```

<a id="s-annex-f-3-3"></a>

#### Annex F.3.3 Preserved OBU types

```text
§   F.3.3.Preserved OBU types

      NOTE: The extraction processes preserve certain OBU types that contain essential configuration
      and metadata, even when their embedded layer identifier (obu_mlayer_id) or temporal layer identifier
      (obu_tlayer_id) would normally cause them to be removed.


    OBU_TEMPORAL_DELIMITER OBUs are unconditionally retained in the sub-bitstream regardless of
    which layers are selected. Temporal delimiters mark the boundaries of temporal units and must be
    preserved to maintain the temporal structure of the extracted sub-bitstream.

    The following OBU types are preserved when obu_mlayer_id is 0 and obu_tlayer_id is 0, provided that
    their extended layer (obu_xlayer_id) is included in the selected operating point:

      • OBU_SEQUENCE_HEADER: Contains sequence-level parameters needed for decoding
      • OBU_LAYER_CONFIGURATION_RECORD: Describes layer structure
      • OBU_ATLAS_SEGMENT: Contains atlas information
      • OBU_OPERATING_POINT_SET: Defines operating points

    If an extended layer is not part of the selected operating point (i.e., not included in the sub-bitstream),
    then all OBUs with that extended layer identifier are removed, including the above OBU types. The
    preservation rule only applies within extended layers that are retained in the sub-bitstream.

                                                                                        ↑ Back to Table of Contents




    AV2 Specification                                                                              Page 1150 of 1169
```
