# AV2 v1.0.0 — § 8. Parsing process

<!-- Verbatim mirror of the AOM AV2 v1.0.0 specification (© Alliance for Open Media). The PDF is normative; this is a faithful `pdftotext -layout` copy. See [./README.md](./README.md) and [./index.md](./index.md). Do not hand-edit: regenerate via scripts/spec/regenerate-av2-spec.sh. -->

<a id="s-8"></a>

## § 8 Parsing process

```text
§   8. Parsing process
```

<a id="s-8-1"></a>

### § 8.1 Parsing process for f(n)

```text
§   8.1. Parsing process for f(n)
    This process is invoked when the descriptor of a syntax element in the syntax tables is equal to f(n).

    The next n bits are read from the bitstream.

    This process is specified as follows:

     x = 0
     for ( i = 0; i < n; i++ ) {
         x = 2 * x + read_bit( )
     }


    read_bit( ) reads the next bit from the bitstream and advances the bitstream position indicator by 1. If the
    bitstream is provided as a series of bytes, then the first bit is given by the most significant bit of the first
    byte.

    The value for the syntax element is given by x.

```

<a id="s-8-2"></a>

### § 8.2 Parsing process for symbol decoder

```text
§   8.2. Parsing process for symbol decoder
```

<a id="s-8-2-1"></a>

#### § 8.2.1 General

```text
§   8.2.1. General

    The entropy decoder is referred to as the "Symbol decoder" and the functions init_symbol( sz ),
    exit_symbol( ), read_symbol( cdf ), and read_bool( ) are used in this specification to indicate the entropy
    decoding operation.

```

<a id="s-8-2-2"></a>

#### § 8.2.2 Initialization process for symbol decoder

```text
§   8.2.2. Initialization process for symbol decoder

    The input to this process is a variable sz specifying the number of bytes to be read by the Symbol
    decoder.

    This process is invoked when the function init_symbol( sz ) is called from the syntax structure.


      NOTE: The bit position will always be byte aligned when init_symbol is invoked because the frame
      header info and the data partitions are always a whole number of bytes long.


    The variable numBits is set equal to Min( sz * 8, 15).

    The variable buf is read using the f(numBits) parsing process.

    The variable paddedBuf is set equal to (buf << (15 - numBits) ).

    The variable SymbolValue is set to ((1 << 15) - 1) ^ paddedBuf.

    The variable SymbolRange is set to 1 << 15.

    The variable SymbolMaxBits is set to 8 * sz - 15.




    AV2 Specification                                                                                 Page 579 of 1169
SymbolMaxBits (when non-negative) represents the number of bits still available to be read. It is allowed for
this number to go negative (either here or during read_symbol or during read_bool). SymbolMaxBits (when
negative) signifies that all available bits have been read, and that -SymbolMaxBits of padding zero bits have
been used in the symbol decoding process. These padding zero bits are not present in the bitstream.

A copy is made of each of the CDF arrays mentioned in the semantics for init_coeff_cdfs and
init_non_coeff_cdfs. The name of the destination for the copy is the name of the CDF array prefixed with
"Tile". The name of the source for the copy is the name of the CDF array with no prefix. This copying
produces the following arrays:

  • TileWarpMvCdf
  • TileTipPredModeCdf
  • TileWarpIdxCdf
  • TileWarpWithMvdCdf
  • TileIsWarpCdf
  • TileUseGdfCdf
  • TileBruModeCdf
  • TileCdefIndex0Cdf
  • TileCdefIndexMinus1With3Cdf
  • TileCdefIndexMinus1With4Cdf
  • TileCdefIndexMinus1With5Cdf
  • TileCdefIndexMinus1With6Cdf
  • TileCdefIndexMinus1With7Cdf
  • TileCdefIndexMinus1With8Cdf
  • TileUseDeltaWarpCdf
  • TileWarpDeltaPrecisionCdf
  • TileWarpDeltaParamLowCdf
  • TileWarpDeltaParamHighCdf
  • TileWarpDeltaParamSignCdf
  • TileYModeSetCdf
  • TileYModeIndexCdf
  • TileYModeOffsetCdf
  • TileCwpIdxCdf
  • TileFscModeCdf
  • TileMrlIndexCdf
  • TileMrlSecIndexCdf
  • TileUseDpcmYCdf
  • TileDpcmModeYCdf
  • TileUseDpcmUvCdf


AV2 Specification                                                                              Page 580 of 1169
  • TileDpcmModeUvCdf
  • TileUVModeCflNotAllowedCdf
  • TileIsCflCdf
  • TileIntrabcCdf
  • TileIntrabcPrecisionCdf
  • TileIntrabcModeCdf
  • TileMorphPredCdf
  • TileRegionTypeCdf
  • TileDoSquareSplitCdf
  • TileDoSplitCdf
  • TileRectTypeCdf
  • TileDoExtPartitionCdf
  • TileDoUneven4wayPartitionCdf
  • TileSegIdExtFlagCdf
  • TileSegmentIdCdf
  • TileSegmentIdExtCdf
  • TileSegmentIdPredictedCdf
  • TileTxPartitionTypeCdf
  • TileTx2or3PartitionTypeCdf
  • TileTxDoPartitionCdf
  • TileLosslessTxSizeCdf
  • TileLosslessInterTxTypeCdf
  • TileSecTxTypeCdf
  • TileMostProbableStxSetCdf
  • TileMostProbableStxSetAdstCdf
  • TileInterpFilterCdf
  • TileUseLocalWarpCdf
  • TileUseExtendWarpCdf
  • TileSingleModeCdf
  • TileUseBawpCdf
  • TileUseBawpChromaCdf
  • TileExplicitBawpCdf
  • TileExplicitBawpScaleCdf
  • TileIsJointCdf
  • TileCompoundModeNonJointCdf
  • TileCompoundModeSameRefsCdf


AV2 Specification                   Page 581 of 1169
  • TileUseOptflowCdf
  • TileTipModeCdf
  • TileUseRefinemvCdf
  • TileDrlModeCdf
  • TileSkipDrlModeCdf
  • TileTipDrlModeCdf
  • TileIsInterCdf
  • TileCompModeCdf
  • TileSkipModeCdf
  • TileSkipCdf
  • TileCompRef0Cdf
  • TileCompRef1Cdf
  • TileSingleRefCdf
  • TileUseMostProbablePrecisionCdf
  • TilePbMvPrecisionCdf
  • TileMvJointAdaptiveCdf
  • TileAmvdIndicesCdf
  • TileJointShellSetCdf
  • TileJointShell0Class0Cdf
  • TileJointShell1Class0Cdf
  • TileJointShell3Class0Cdf
  • TileJointShell4Class0Cdf
  • TileJointShell5Class0Cdf
  • TileJointShell6Class0Cdf
  • TileJointShell0Class1Cdf
  • TileJointShell1Class1Cdf
  • TileJointShell3Class1Cdf
  • TileJointShell4Class1Cdf
  • TileJointShell5Class1Cdf
  • TileJointShell6Class1Cdf
  • TileJointShellLastTwoClassesCdf
  • TileShellOffsetLowClassCdf
  • TileShellOffsetOtherClassCdf
  • TileColMvGreaterCdf
  • TileColMvIndexCdf
  • TileJmvdScaleModeCdf


AV2 Specification                     Page 582 of 1169
  • TileJmvdAdaptiveScaleModeCdf
  • TilePaletteYModeCdf
  • TileIdentityRowYCdf
  • TilePaletteYSizeCdf
  • TilePaletteSize2YColorCdf
  • TilePaletteSize3YColorCdf
  • TilePaletteSize4YColorCdf
  • TilePaletteSize5YColorCdf
  • TilePaletteSize6YColorCdf
  • TilePaletteSize7YColorCdf
  • TilePaletteSize8YColorCdf
  • TileDeltaQCdf
  • TileIntraTxTypeLongCdf
  • TileInterTxTypeLongCdf
  • TileIsLongSideDctCdf
  • TileIntraTxTypeSet1Cdf
  • TileIntraTxTypeSet2Cdf
  • TileInterTxTypeSet1Cdf
  • TileInterTxTypeSet2Cdf
  • TileInterTxTypeSet3Cdf
  • TileInterTxTypeSet4Cdf
  • TileInterTxTypeIndexSet1Cdf
  • TileInterTxTypeIndexSet2Cdf
  • TileInterTxTypeOffsetSet1Cdf
  • TileInterTxTypeOffsetSet2Cdf
  • TileInterIntraCdf
  • TileWarpInterIntraCdf
  • TileCflSignCdf
  • TileWedgeInterIntraCdf
  • TileCompGroupIdxCdf
  • TileCompoundTypeCdf
  • TileInterIntraModeCdf
  • TileWedgeAngleDirCdf
  • TileWedgeAngle0Cdf
  • TileWedgeAngle1Cdf
  • TileWedgeDist1Cdf


AV2 Specification                  Page 583 of 1169
  • TileWedgeDist2Cdf
  • TileCflAlphaCdf
  • TileCflIndexCdf
  • TileCflMhDirCdf
  • TileCflMhccpCdf
  • TileUseAmvdCdf
  • TileCcsoBlkCdf
  • TileUseWienerNsCdf
  • TileWienerNsLengthCdf
  • TileWienerNsUvSymCdf
  • TileWienerNsBaseCdf
  • TileUsePcWienerCdf
  • TileFlexRestorationTypeCdf
  • TileTxbSkipCdf
  • TileCctxTypeCdf
  • TileEobPt16Cdf
  • TileEobPt32Cdf
  • TileEobPt64Cdf
  • TileEobPt128Cdf
  • TileEobPt256Cdf
  • TileEobPt512Cdf
  • TileEobPt1024Cdf
  • TileEobExtraCdf
  • TileDcSignCdf
  • TileVTxbSkipCdf
  • TileCoeffBaseEobCdf
  • TileCoeffBaseLfEobCdf
  • TileCoeffBaseCdf
  • TileCoeffBaseLfCdf
  • TileCoeffBasePhCdf
  • TileCoeffBrCdf
  • TileCoeffBrLfCdf
  • TileCoeffBrUvCdf
  • TileCoeffBaseLfUvCdf
  • TileCoeffBaseLfEobUvCdf
  • TileCoeffBaseUvCdf


AV2 Specification                Page 584 of 1169
      • TileCoeffBaseEobUvCdf
      • TileCoeffBaseBobCdf
      • TileCoeffBrIdtxCdf
      • TileCoeffBaseIdtxCdf
      • TileIdtxSignCdf
      • TileUseDipCdf
      • TileDipModeCdf

```

<a id="s-8-2-3"></a>

#### § 8.2.3 Boolean decoding process

```text
§   8.2.3. Boolean decoding process

    This process decodes a pseudo-raw bit assuming equal probability for decoding a 0 or a 1.

    This process is invoked when the function read_bool( ) is called from the read_literal function in § 8.2.5
    Parsing process for read_literal.

    The variables cur and symbol are calculated as follows:

     cur = SymbolRange >> 1
     symbol = SymbolValue < cur


    If symbol is equal to 0, SymbolValue is set equal to SymbolValue - cur.

    The range and value are renormalized by the following ordered steps:

     1. The variable numBits is set equal to Clip3(0, 1, SymbolMaxBits). This represents the number of new bits
        to read from the bitstream.
     2. The variable newData is read using the f(numBits) parsing process.
     3. The variable SymbolValue is set to (SymbolValue << 1) | (newData ^ 1).
     4. The variable SymbolMaxBits is set to SymbolMaxBits - 1.

    The return value from the function is given by symbol.

```

<a id="s-8-2-4"></a>

#### § 8.2.4 Exit process for symbol decoder

```text
§   8.2.4. Exit process for symbol decoder

    This process is invoked when the function exit_symbol( ) is called from the syntax structure.

    It is a requirement of bitstream conformance that SymbolMaxBits is greater than or equal to -14
    whenever this process is invoked.

    The variable trailingBitPosition is set equal to get_position() - Min(15, SymbolMaxBits+15).

    The bitstream position indicator is advanced by Max(0,SymbolMaxBits). (This skips over any trailing bits
    that have not already been read during symbol decode.)

    The variable paddingEndPosition is set equal to get_position().


      NOTE: paddingEndPosition will always be a multiple of 8 indicating that the bit position is byte
      aligned.




    AV2 Specification                                                                               Page 585 of 1169
It is a requirement of bitstream conformance that the bit at position trailingBitPosition is equal to 1.

It is a requirement of bitstream conformance that the bit at position x is equal to 0 for values of x strictly
between trailingBitPosition and paddingEndPosition.


  NOTE:       This exit process consumes the OBU trailing bits for a Tile Group.


The variable numLog2 (specifying the base 2 logarithm of the number of tiles used in CDF averaging) is
set equal to Min( 3, FloorLog2( TileCols * TileRows ) ).

The variables copyCdf and avgCdf (specifying whether to copy or average the CDFs) are set as follows:

 copyCdf = 0
 avgCdf = 0
 if ( enable_avg_cdf && avg_cdf_type ) {
     avgCdf = TileNum < 1 << numLog2
 } else {
     copyCdf = ( TileNum == context_update_tile_id )
 }


If copyCdf is equal to 1, a copy is made of the final CDF values for each of the CDF arrays mentioned in
the semantics for init_coeff_cdfs and init_non_coeff_cdfs. The name of the destination for the copy is the
name of the CDF array prefixed with "Saved". The name of the source for the copy is the name of the
CDF array prefixed with "Tile". For example, an array SavedIdentityRowYCdf will be created with values
equal to TileIdentityRowYCdf.

If avgCdf is equal to 1, a copy with averaging is made of the final CDF values for each of the CDF arrays
mentioned in the semantics for init_coeff_cdfs and init_non_coeff_cdfs. The name of the destination is the
name of the CDF array prefixed with "Saved". The name of the source is the name of the CDF array
prefixed with "Tile". For example, an array SavedIdentityRowYCdf will be created based on values from
TileIdentityRowYCdf.

The copy with averaging works for each CDF of the cdf array in turn by calling the avg_cdf function with a
reference to the destination array, a reference to the source array, and the length of each CDF as inputs.

For example, the array SavedIdentityRowYCdf will be created as follows:

 for( i = 0; i < PALETTE_ROW_FLAG_CONTEXTS; i++ ) {
     avg_cdf( SavedIdentityRowYCdf[ i ], IdentityRowYCdf[ i ], 4 )
 }


The avg_cdf function (which updates the destination CDF) is specified as:


 avg_cdf( cdf, tilecdf, sz ) {
     if ( TileNum == 0 ) {
         for( i = 0; i < sz - 2; i++ ) {
             cdf[i] = 1 << 15
         }
         cdf[ sz - 2 ] = tilecdf[ sz - 2 ]
         cdf[ sz - 1 ] = 0
     }
     for( i = 0; i < sz - 2; i++ ) {




AV2 Specification                                                                               Page 586 of 1169
              cdf[ i ] -= ( (1 << 15) - tilecdf[ i ] ) >> numLog2
          }
          cdf[ sz - 1 ] += tilecdf[ sz - 1 ] >> numLog2



      NOTE: The cdf[ sz - 2 ] element contains the rate and is copied from the first tile. The cdf[ sz - 1 ]
      element contains the activation count and is averaged across the tiles. The other elements contain
      CDF values.

```

<a id="s-8-2-5"></a>

#### § 8.2.5 Parsing process for read_literal

```text
§   8.2.5. Parsing process for read_literal

    This process is invoked when the function read_literal( n ) is invoked.

    This process is specified as follows:

     FrameSymbolCount += n
     x = 0
     for ( i = 0 ; i < n; i++ ) {
         x = 2 * x + read_bool( )
     }


    The return value for the function is given by x.

```

<a id="s-8-2-6"></a>

#### § 8.2.6 Symbol decoding process

```text
§   8.2.6. Symbol decoding process

    The input to this process is an array cdf of length N + 1 which specifies the cumulative distribution for a
    symbol with N possible values.

    The output of this process is the variable symbol, containing a decoded syntax element. The process also
    modifies the input array cdf to adapt the probabilities to the content of the stream.

    This process is invoked when the function read_symbol( cdf ) is called.


      NOTE: When this process is invoked, N will be greater than 1. cdf[ N-1 ] contains a constant that
      defines the rate of adaption. cdf[N] contains a count of the number of times this cdf has been used (up
      to a maximum of 32).


    The variables cur, prev, and symbol are calculated as follows:

     FrameSymbolCount++
     cur = SymbolRange
     symbol = -1
     do {
          symbol++
          prev = cur
          if (symbol == N - 1) {
              f = 0
          } else {
              f = ( 1 << 15 ) - cdf[ symbol ]
          }
          pp = ((f >> EC_PROB_SHIFT) << 4) + Prob_Inc[ N - 2 ][ symbol ]
          cur = ( ( (SymbolRange >> 8) * pp) >> 7 ) << 3
     } while ( SymbolValue < cur )



      NOTE:       Implementations may prefer to store the inverse cdf to move the subtraction out of this loop.



    AV2 Specification                                                                             Page 587 of 1169
    The variable newRange is set equal to prev - cur.

    The variable newValue is set equal to SymbolValue - cur.

    The range and value are renormalized by the following ordered steps:

     1. The variable bits is set to 15 – FloorLog2( newRange ). This represents the number of new bits to be
        added to SymbolValue.
     2. The variable SymbolRange is set equal to newRange << bits.
     3. The variable numBits is set equal to Clip3(0, bits, SymbolMaxBits). This represents the number of new
        bits to read from the bitstream.
     4. The variable newData is read using the f(numBits) parsing process.
     5. The variable paddedData is set equal to newData << ( bits - numBits ).
     6. The variable mask is set equal to (1 << bits) - 1.
     7. The variable SymbolValue is set to (newValue << bits) | (paddedData ^ mask).
     8. The variable SymbolMaxBits is set to SymbolMaxBits - bits.


      NOTE:       bits may be equal to 0, in which case these ordered steps have no effect.


    If disable_cdf_update is equal to 0, the cumulative distribution is updated as follows:

     timeInterval = cdf[ N ] > 31 ? 2 : cdf[ N ] > 15 ? 1 : 0
     rate = 3 + timeInterval + Min( FloorLog2( N ), 2 ) +
            Para_Adjustment_List[cdf[N - 1]][timeInterval]
     for ( i = 0; i < N - 1; i++ ) {
         if ( i < symbol ) {
             cdf[ i ] -= cdf[ i ] >> rate
         } else {
             cdf[ i ] += ( ( 1 << 15 ) - cdf[ i ] ) >> rate
         }
     }
     cdf[ N ] += ( cdf[ N ] < 32 )



      NOTE: The last entry of the cdf array is used to keep a count of the number of times the symbol has
      been decoded (up to a maximum of 32). This allows the cdf adaption rate to depend on the number of
      times the symbol has been decoded.


      NOTE:       The penultimate entry of the cdf array holds the (constant) base adaption rate for the cdf.


    The return value from the function is given by symbol.

```

<a id="s-8-3"></a>

### § 8.3 Parsing process for CDF encoded syntax elements

```text
§   8.3. Parsing process for CDF encoded syntax elements
```

<a id="s-8-3-1"></a>

#### § 8.3.1 General

```text
§   8.3.1. General

    This process is invoked when the descriptor of a syntax element in the syntax tables is equal to S.

    The input to this process is the name of a syntax element.




    AV2 Specification                                                                              Page 588 of 1169
    § 8.3.2 Cdf selection process specifies how a CDF array is chosen for the syntax element. The variable cdf
    is set equal to a reference to this CDF array.


      NOTE:       The array must be passed by reference because read_symbol will adjust the array contents.


    The output of this process is the result of calling the function read_symbol( cdf ).

```

<a id="s-8-3-2"></a>

#### § 8.3.2 Cdf selection process

```text
§   8.3.2. Cdf selection process

    The input to this process is the name of a syntax element.

    The output of this process is a reference to a CDF array.

    When the description in this section uses variables, these variables are taken to have the values defined
    by the syntax tables at the point that the syntax element is being decoded.

    The probabilities depend on the syntax element as follows:

    use_intrabc: The cdf for use_intrabc is given by TileIntrabcCdf[ ctx ] where ctx is computed as follows:

     ctx = 0
     for(n = 0; n < NNum; n++) {
         if ( RefFrames[NPos[n][0]][NPos[n][1]][0] == INTRA_FRAME &&
              IsInters[NPos[n][0]][NPos[n][1]] ) {
             ctx += 1
         }
     }


    intrabc_mode: The cdf for intrabc_mode is given by TileIntrabcModeCdf.

    intrabc_precision: The cdf for intrabc_precision is given by TileIntrabcPrecisionCdf.

    morph_pred: The cdf for morph_pred is given by TileMorphPredCdf[ctx] where ctx is computed as
    follows:

     ctx = 0
     for( n = 0; n < NNum; n++ ) {
         ctx += MorphPreds[ NPos[ n ][ 0 ] ][ NPos[ n ][ 1 ] ]
     }


    tip_pred_mode: The cdf is given by TileTipPredModeCdf.

    is_warp: The cdf is given by TileIsWarpCdf[WarpMvCount].

    use_gdf: The cdf is given by TileUseGdfCdf.

    bru_mode: The cdf is given by TileBruModeCdf.

    warp_mv: The cdf is given by TileWarpMvCdf.

    warp_idx: The cdf is given by TileWarpIdxCdf[idx].

    warpmv_with_mvd: The cdf is given by TileWarpWithMvdCdf.

    y_mode_set: The cdf for y_mode_set is given by TileYModeSetCdf.


    AV2 Specification                                                                            Page 589 of 1169
y_mode_index: The cdf for y_mode_index is given by TileYModeIndexCdf[ ctx ] where ctx is computed as
follows:

 ctx = (get_joint_mode(0) >= NON_DIRECTIONAL_MODES_COUNT) +
       (get_joint_mode(1) >= NON_DIRECTIONAL_MODES_COUNT)


y_mode_offset: y_mode_offset uses the same derivation for the variable ctx as for the syntax element
y_mode_index.

The cdf for y_mode_offset is given by TileYModeOffsetCdf[ ctx ].

uv_mode: The variable ctx is set equal to is_directional_mode(YMode).

The cdf for uv_mode is given by TileUVModeCflNotAllowedCdf[ ctx ].

is_cfl: The cdf is given by TileIsCflCdf[ ctx ] where ctx is computed as follows:

 ctx = 0
 if ( AvailUChroma && UVCfls[ ChromaMiRow - 1 ][ ChromaMiCol ] )
     ctx += 1
 if ( AvailLChroma && UVCfls[ ChromaMiRow ][ ChromaMiCol - 1] )
     ctx += 1


cwp_idx: The cdf is given by TileCwpIdxCdf[idx].

fsc_mode: The cdf is given by TileFscModeCdf[ ctx ][ Fsc_Bsize_Groups[ MiSize ] ] where ctx is
computed as follows:

 if ( FrameIsIntra || RegionType == INTRA_REGION ) {
     ctx = 0
     for( n = 0; n < NNum; n++ ) {
          ctx += FscModes[ NPos[ n ][ 0 ] ][ NPos[ n ][ 1 ] ]
     }
 } else {
     ctx = 3
 }


and the constant table Fsc_Bsize_Groups is defined as:

 Fsc_Bsize_Groups[BLOCK_SIZES] = {
     0, 1, 1, 2, 3, 3, 4, 5, 5, 5, 6, 6, 6, 6, 6, 6,
     6, 6, 6, 3, 3, 4, 4, 6, 6, 4, 4, 6, 6
 }


mrl_index: The cdf is given by TileMrlIndexCdf[ctx] where ctx is computed as follows:

 ctx = 0
 for( n = 0; n < NNum; n++ ) {
     ctx += UsesMrls[ NPos[ n ][ 0 ] ][ NPos[ n ][ 1 ] ] > 0
 }




AV2 Specification                                                                         Page 590 of 1169
mrl_sec_index: The cdf is given by TileMrlSecIndexCdf[ctx] where ctx is computed as follows:

 ctx = 0
 for(n = 0; n < NNum; n++) {
     ctx += UsesMrls[ NPos[ n ][ 0 ] ][ NPos[ n ][ 1 ] ] == 2
 }


use_dpcm_y: The cdf is given by TileUseDpcmYCdf.

dpcm_mode_y: The cdf is given by TileDpcmModeYCdf.

use_dpcm_uv: The cdf is given by TileUseDpcmUvCdf.

dpcm_mode_uv: The cdf is given by TileDpcmModeUvCdf.

region_type: The cdf is given by TileRegionTypeCdf[ ctx ] where ctx is computed as follows:

 numSamples = (num4x4wide * 4) * (num4x4high * 4)
 if (numSamples <= 128)
      ctx = 0
 else if (numSamples <= 512)
      ctx = 1
 else if (numSamples <= 1024)
      ctx = 2
 else
      ctx = 3


cdef_index0: The cdf is given by TileCdefIndex0Cdf[ ctx ] where ctx is computed as follows:

 ctx = 0
 cnt = 0
 leftCol = (MiCol - cdefSize4) & cdefMask4
 leftRow = MiRow & cdefMask4
 if ( leftCol >= MiColStart ) {
     ctx += cdef_idx[ leftRow ][leftCol ] == 0
     cnt += 1
 }
 aboveCol = MiCol & cdefMask4
 aboveRow = (MiRow - cdefSize4) & cdefMask4
 shift = Mi_Width_Log2[ SbSize ]
 curSbRow = MiRow >> shift
 aboveSbRow = aboveRow >> shift
 if ( aboveRow >= MiRowStart && aboveSbRow == curSbRow ) {
     ctx += cdef_idx[ aboveRow ][aboveCol ] == 0
     cnt += 1
 }
 if ( ctx != 0 && cnt == ctx ) {
     ctx += 1
 }


cdef_index_minus_1: The cdf is given as follows:

  • If CdefStrengths is equal to 3, the cdf is given by TileCdefIndexMinus1With3Cdf.
  • If CdefStrengths is equal to 4, the cdf is given by TileCdefIndexMinus1With4Cdf.
  • If CdefStrengths is equal to 5, the cdf is given by TileCdefIndexMinus1With5Cdf.
  • If CdefStrengths is equal to 6, the cdf is given by TileCdefIndexMinus1With6Cdf.
  • If CdefStrengths is equal to 7, the cdf is given by TileCdefIndexMinus1With7Cdf.



AV2 Specification                                                                         Page 591 of 1169
  • Otherwise (CdefStrengths is equal to 8), the cdf is given by TileCdefIndexMinus1With8Cdf.

do_split: The variable ctx is computed as follows:

 bsw = Max(Mi_Width_Log2[ bSize ], 1)
 bsh = Max(Mi_Height_Log2[ bSize ], 1)
 ctx1 = Mi_Height_Log2[ LeftMiSizes[ PlaneStart ][ r ] ] < bsh
 ctx2 = Mi_Width_Log2[ AboveMiSizes[ PlaneStart ][ c ] ] < bsw
 ctx = Partition_Size_Adjust[ bSize ] * 4 + ctx1 * 2 + ctx2


where Partition_Size_Adjust is defined as:

 Partition_Size_Adjust[BLOCK_SIZES] = {
     0, 0, 0, 0, 1, 1, 1, 2,
     2, 2, 3, 3, 3, 4, 5, 6,
     7, 8, 9, 10, 11, 12, 13, 14,
     15, 0, 0, 0, 0
 }


The cdf for do_split is given by TileDoSplitCdf[ PlaneStart ][ ctx ].

do_square_split: The cdf is given by TileDoSquareSplitCdf[ PlaneStart ][ ctx ] where ctx is computed as
follows:

 bsw = Mi_Width_Log2[ bSize ]
 bsh = Mi_Height_Log2[ bSize ]
 above = AvailU && ( Mi_Width_Log2[ MiSizes[ PlaneStart ][ r - 1 ][ c ] ] < bsw )
 left = AvailL && ( Mi_Height_Log2[ MiSizes[ PlaneStart ][ r ][ c - 1 ] ] < bsh )
 ctx = (bSize == BLOCK_256X256 ? 4 : 0) + left * 2 + above



  NOTE: PlaneStart will always be equal to 0 for do_square_split as the chroma partition is forced for
  large block sizes.


rect_type: The variable ctx is computed as follows:

 bsw = Max(Mi_Width_Log2[ bSize ], 1)
 bsh = Max(Mi_Height_Log2[ bSize ], 1)
 ctx1 = Mi_Height_Log2[ LeftMiSizes[ PlaneStart ][ r ] ] < bsh
 ctx2 = Mi_Width_Log2[ AboveMiSizes[ PlaneStart ][ c ] ] < bsw
 ctx = Partition_Size_Adjust_Rect_Type[ bSize ] * 4 + ctx1 * 2 + ctx2


where Partition_Size_Adjust_Rect_Type is defined as:

 Partition_Size_Adjust_Rect_Type[ BLOCK_SIZES ] = {
     0, 0, 0, 0, 1, 2, 0, 1,
     2, 3, 4, 5, 6, 7, 8, 9,
     10, 11, 12, 13, 14, 13, 14, 13,
     14, 0, 0, 0, 0
 }


The cdf for rect_type is given by TileRectTypeCdf[ PlaneStart ][ ctx ].




AV2 Specification                                                                         Page 592 of 1169
do_ext_partition: The variable ctx is computed as follows:

 if (rectType == RECT_HORZ) {
     bsh = Max(Mi_Height_Log2[ bSize ] - 1, 1)
     ctx1 = Mi_Height_Log2[ LeftMiSizes[ PlaneStart ][ r ] ] < bsh
     ctx2 = Mi_Height_Log2[
                LeftMiSizes[ PlaneStart ]
                            [ r + (Num_4x4_Blocks_High[ bSize ] >> 1) ] ] < bsh
 } else {
     bsw = Max(Mi_Width_Log2[ bSize ] - 1, 1)
     ctx1 = Mi_Width_Log2[ AboveMiSizes[ PlaneStart ][ c ] ] < bsw
     ctx2 = Mi_Width_Log2[
                AboveMiSizes[ PlaneStart ]
                             [ c + (Num_4x4_Blocks_Wide[ bSize ] >> 1) ] ] < bsw
 }
 adjSize = Partition_Size_Adjust[ bSize ]
 ctx = adjSize * 4 + ctx1 * 2 + ctx2


The cdf for do_ext_partition is given by TileDoExtPartitionCdf[ PlaneStart ][ ctx ].

do_uneven_4way_partition: do_uneven_4way_partition uses the same derivation for the variable ctx as
for the syntax element do_ext_partition.

The cdf for do_uneven_4way_partition is given by TileDoUneven4wayPartitionCdf[ PlaneStart ][ ctx ].

tx_do_partition: the cdf is given by TileTxDoPartitionCdf[fsc_mode][is_inter]
[Size_To_Tx_Part_Group_Lookup[MiSize]].

tx_2or3_partition_type: the cdf is given by TileTx2or3PartitionTypeCdf[fsc_mode][is_inter]
[Size_To_Tx_Type_Group_Vert_Or_Horz[MiSize] - 1].

tx_partition_type: the cdf is given by TileTxPartitionTypeCdf[fsc_mode][is_inter]
[Size_To_Tx_Type_Group_Vert_And_Horz[MiSize]].

lossless_inter_tx_type: the cdf is given by TileLosslessInterTxTypeCdf.

lossless_tx_size: the cdf is given by TileLosslessTxSizeCdf[Size_Group[MiSize]][is_inter].

sec_tx_type: The cdf is given by TileSecTxTypeCdf[ is_inter ][Tx_Size_Sqr[ txSz ]].

most_probable_stx_set: The cdf is given as follows:

  • If TxType is equal to ADST_ADST and Tx_Width[ txSz ] is greater than or equal to 8 and
    Tx_Height[ txSz ] is greater than or equal to 8, the cdf is given by TileMostProbableStxSetAdstCdf.
  • Otherwise, the cdf is given by TileMostProbableStxSetCdf.

seg_id_ext_flag: The cdf is given by TileSegIdExtFlagCdf[ ctx ], where the variable ctx is computed by:

 if ( prevUL < 0 )
      ctx = 0
 else if ( (prevUL == prevU) && (prevUL == prevL) )
      ctx = 2
 else if ( (prevUL == prevU) || (prevUL == prevL) || (prevU == prevL) )
      ctx = 1
 else
      ctx = 0




AV2 Specification                                                                            Page 593 of 1169
segment_id: if seg_id_ext_flag is equal to 0, the cdf is given by TileSegmentIdCdf[ ctx ]. Otherwise, the
cdf is given by TileSegmentIdExtCdf[ ctx ].

The variable ctx is computed by:

 if ( prevUL < 0 )
      ctx = 0
 else if ( (prevUL == prevU) && (prevUL == prevL) )
      ctx = 2
 else if ( (prevUL == prevU) || (prevUL == prevL) || (prevU == prevL) )
      ctx = 1
 else
      ctx = 0


seg_id_predicted: the cdf is given by TileSegmentIdPredictedCdf[ ctx ], where ctx is computed by:

 ctx = LeftSegPredContext[ MiRow ] + AboveSegPredContext[ MiCol ]


single_mode: the cdf is given by TileSingleModeCdf[ NewMvContext ].

use_most_probable_precision: the cdf is given by TileUseMostProbablePrecisionCdf[ ctx ] where ctx is
computed by:

 ctx = 0
 for( n = 0; n < NNum; n++ ) {
     if ( UseMostProbablePrecisions[ NPos[n][0] ][ NPos[n][1] ] ) {
         ctx += 1
     }
 }


pb_mv_precision: the cdf is given by TilePbMvPrecisionCdf[ctx][FrameMvPrecision -
MV_PRECISION_HALF_PEL] where ctx is computed by:

 ctx = 0
 for( n = 0; n < NNum; n++ ) {
     if ( MvPrecisions[ NPos[n][0] ][ NPos[n][1] ] < FrameMvPrecision ) {
         ctx = 1
     }
 }


jmvd_scale_mode: if use_amvd is equal to 1, the cdf is given by TileJmvdAdaptiveScaleModeCdf.
Otherwise, the cdf is given by TileJmvdScaleModeCdf.

use_bawp: the cdf is given by TileUseBawpCdf.

use_bawp_chroma: the cdf is given by TileUseBawpChromaCdf.

explicit_bawp: the cdf is given by TileExplicitBawpCdf[ctx] where ctx is computed by:

 ctx = (YMode == NEARMV) ? 0 : (YMode == NEWMV && use_amvd ? 1 : 2)


explicit_bawp_scale: the cdf is given by TileExplicitBawpScaleCdf.




AV2 Specification                                                                            Page 594 of 1169
use_amvd: the cdf is given by TileUseAmvdCdf[index][ctx] where index and ctx are computed by:

 if ( YMode == NEAR_NEWMV ) {
     index = use_optflow ? 2 : 0
 } else if ( YMode == NEW_NEARMV ) {
     index = use_optflow ? 3 : 1
 } else if ( YMode == NEWMV ) {
     index = 4
 } else if ( YMode == JOINT_NEWMV ) {
     index = use_optflow ? 6 : 5
 } else { // NEW_NEWMV
     index = use_optflow ? 8 : 7
 }
 ctx = 0
 for(n = 0; n < NNumBuf; n++) {
     ctx += NRefFrame[n][0] == RefFrame[0] &&
            UsesAmvds[NPosBuf[n][0]][NPosBuf[n][1]]
 }


drl_mode: If RefFrame[0] is equal to TIP_FRAME, the cdf is given by TileTipDrlModeCdf[ Min(idx, 2) ].
Otherwise, if skip_mode is equal to 1, the cdf is given by TileSkipDrlModeCdf[ Min(idx, 2) ]. Otherwise
(skip_mode is equal to 0 and RefFrame[0] is not equal to TIP_FRAME), the cdf is given by
TileDrlModeCdf[ Min(idx, 2) ][ NewMvContext ].

is_inter: the cdf is given by TileIsInterCdf[ ctx ] where ctx is computed by:

 if ( NNumBuf == 2 )
      ctx = ( NIntra[ 0 ] && NIntra[ 1 ] ) ? 3 : NIntra[ 0 ] || NIntra[ 1 ]
 else if ( NNumBuf == 1 )
      ctx = 2 * NIntra[ 0 ]
 else
      ctx = 0


dip_mode: the cdf is given by TileDipModeCdf.

use_dip: the cdf is given by TileUseDipCdf[ ctx ] where ctx is computed as follows:

 ctx = 0
 for( n = 0; n < NNum; n++ ) {
     ctx += UseDip[ NPos[ n ][ 0 ] ][ NPos[ n ][ 1 ] ]
 }


tip_mode: the cdf is given by TileTipModeCdf[ ctx ] where ctx is computed as follows:

 ctx = 0
 for( n = 0; n < NNumBuf; n++ ) {
     ctx += NRefFrame[ n ][ 0 ] == TIP_FRAME
 }


comp_mode: the cdf is given by TileCompModeCdf[ ctx ] where ctx is computed by:

 if ( NNumBuf == 2 ) {
     if ( NSingle[0] && NSingle[1] )
         ctx = check_backward( NRefFrame[ 0 ][ 0 ] ) ^
               check_backward( NRefFrame[ 1 ][ 0 ] )
     else if ( NSingle[0] )
         ctx = 2 + ( check_backward( NRefFrame[ 0 ][ 0 ] ) || NIntra[ 0 ] )
     else if ( NSingle[1] )



AV2 Specification                                                                          Page 595 of 1169
             ctx = 2 + ( check_backward( NRefFrame[ 1 ][ 0 ] ) || NIntra[ 1 ] )
      else
          ctx = 4
 } else if ( NNumBuf == 1 ) {
     if ( NSingle[ 0 ] )
          ctx = check_backward( NRefFrame[ 0 ][ 0 ] )
     else
          ctx = 3
 } else {
     ctx = 1
 }


where check_backward is a function specified as follows:

 check_backward(refFrame) {
   if ( refFrame == TIP_FRAME ) {
     return 1
   }
   return is_inter_ref_frame(refFrame) && FrameDistance[refFrame] < 0
 }


skip_mode: the cdf is given by TileSkipModeCdf[ ctx ] where ctx is computed by:

 ctx = 0
 for( n = 0; n < NNumBuf; n++ ) {
     ctx += SkipModes[ NPosBuf[n][0] ][ NPosBuf[n][1] ]
 }


skip_flag: the cdf is given by TileSkipCdf[ ctx ] where ctx is computed by:

 ctx = 0
 for( n = 0; n < NNumBuf; n++ ) {
     ctx += Skips[ NPosBuf[n][0] ][ NPosBuf[n][1] ]
 }
 if (skip_mode) {
     ctx += (SKIP_CONTEXTS >> 1)
 }


comp_ref: if nFound is equal to 0, the cdf is given by TileCompRef0Cdf[ ctx ][ ref ]. Otherwise, the cdf is
given by TileCompRef1Cdf[ ctx ][ bitType ][ ref ] where bitType is equal to
(FrameDistance[ RefFrame[ 0 ] ] >= 0) ^ (FrameDistance[ ref ] >= 0). The variable ctx is computed by:

 thisRefCount = count_refs(ref)
 nextRefsCount = 0
 for ( i = ref + 1; i < NumTotalRefs; i++) {
     nextRefsCount += count_refs(i)
 }
 if (thisRefCount == nextRefsCount) {
     ctx = 1
 } else if (thisRefCount < nextRefsCount) {
     ctx = 0
 } else {
     ctx = 2
 }




AV2 Specification                                                                             Page 596 of 1169
where count_refs is defined as:

 count_refs(frameType) {
     c = 0
     for( n = 0; n < NNumBuf; n++ ) {
         for( list = 0; list < 2; list++ ) {
              if ( NRefFrame[ n ][ list ] == frameType ) c++
         }
     }
     return c
 }


single_ref: the cdf is given by TileSingleRefCdf[ ctx ][ ref ] where ctx is computed as in the CDF
selection process for comp_ref.

is_joint: the cdf is given by TileIsJointCdf[ctx] where ctx is computed by:

 firstDist = Abs(get_relative_dist( OrderHints[ RefFrame[ 0 ] ], OrderHint ))
 secondDist = Abs(get_relative_dist( OrderHints[ RefFrame[ 1 ] ], OrderHint ))
 ctx = is_same_side() || firstDist != secondDist ||
                 (OrderHints[ RefFrame[ 0 ] ] == RESTRICTED_OH) !=
                 (OrderHints[ RefFrame[ 1 ] ] == RESTRICTED_OH)


compound_mode_non_joint: the cdf is given by TileCompoundModeNonJointCdf[ NewMvContext ].

compound_mode_same_refs: the cdf is given by TileCompoundModeSameRefsCdf[ NewMvContext ].

use_optflow: the cdf is given by TileUseOptflowCdf[ YMode != NEAR_NEARMV ].

use_refinemv: the cdf is given by TileUseRefinemvCdf[ ctx ] where ctx is computed as follows:

 ctx = 1 + (YMode - NEAR_NEARMV) + 6 * use_optflow
 if (use_optflow && YMode > GLOBAL_GLOBALMV) {
     ctx -= 1
 }


interp_filter: the cdf is given by TileInterpFilterCdf[ ctx ] where ctx is computed by:

 ctx = is_inter_ref_frame( RefFrame[ 1 ] ) * 4
 leftType = 3
 aboveType = 3

 if ( NNum > 0 ) {
     if ( RefFrames[ NPos[0][0] ][ NPos[0][1] ][ 0 ] == RefFrame[ 0 ] ||
         RefFrames[ NPos[0][0] ][ NPos[0][1] ][ 1 ] == RefFrame[ 0 ] )
         leftType = InterpFilters[ NPos[0][0] ] [ NPos[0][1] ]
 }
 if ( NNum > 1 ) {
     if ( RefFrames[ NPos[1][0] ][ NPos[1][1] ][ 0 ] == RefFrame[ 0 ] ||
         RefFrames[ NPos[1][0] ][ NPos[1][1] ][ 1 ] == RefFrame[ 0 ] )
         aboveType = InterpFilters[ NPos[1][0] ] [ NPos[1][1] ]
 }

 if ( leftType == aboveType )
     ctx += leftType
 else if ( leftType == 3 )
     ctx += aboveType
 else if ( aboveType == 3 )




AV2 Specification                                                                            Page 597 of 1169
        ctx += leftType
 else
        ctx += 3


use_local_warp: the cdf is given by TileUseLocalWarpCdf[ ctx ] where ctx is computed by:

 ctx = 0
 hasWarp = 0
 for( n = 0; n < NNum; n++ ) {
     m = MotionModes[ NPos[n][0] ][ NPos[n][1] ]
     if ( m >= LOCALWARP ) {
         hasWarp = 1
     }
     if ( m == LOCALWARP ) {
         ctx += 1
     }
 }
 ctx += hasWarp


use_extend_warp: the cdf is given by TileUseExtendWarpCdf[ ctx ] where ctx is computed by:

 ctx = 0
 for( n = 0; n < NNum; n++ ) {
     if ( MotionModes[ NPos[n][0] ][ NPos[n][1] ] >= LOCALWARP ) {
         ctx += 1
     }
 }


mv_joint: the cdf is given by TileMvJointAdaptiveCdf.

amvd_index: the cdf is given by TileAmvdIndicesCdf[ comp ].

shell_set: the cdf is given by TileJointShellSetCdf[ MvCtx ].

shell_class: the cdf is given by TileJointShellPClassQCdf[ MvCtx ].

where Q is equal to the value of shell_set and P is equal to the value of MvPrecision (P will be between 0
and 6 inclusive, except 2 is not reachable).

joint_shell_last_two_classes: the cdf is given by TileJointShellLastTwoClassesCdf[ MvCtx ].

shell_offset_low_class: the cdf is given by TileShellOffsetLowClassCdf[ MvCtx ][ shellClass ].

shell_offset_class2: the cdf is given by TileShellOffsetClass2Cdf[ MvCtx ].

shell_offset_other_class: the cdf is given by TileShellOffsetOtherClassCdf[ MvCtx ][ i ].

col_mv_greater: the cdf is given by TileColMvGreaterCdf[ MvCtx ][ i ].

col_mv_index: the cdf is given by TileColMvIndexCdf[ MvCtx ][ Min(shellClass,
NUM_CTX_COL_MV_INDEX - 1) ].

all_zero: the variable ctx is computed as follows:

 maxX4 = MiCols
 maxY4 = MiRows
 if ( plane > 0 ) {
     maxX4 = maxX4 >> SubsamplingX



AV2 Specification                                                                            Page 598 of 1169
      maxY4 = maxY4 >> SubsamplingY
 }

 w = Tx_Width[txSz]
 h = Tx_Height[txSz]

 bsize = get_plane_residual_size( plane > 0 ? ChromaMiSize : MiSize, plane )
 bw = Block_Width[ bsize ]
 bh = Block_Height[ bsize ]

 if ( plane == 0 ) {
     top = 0
     left = 0
     for ( k = 0; k < w4; k++ ) {
          if ( x4 + k < maxX4 )
              top |= AboveLevelContext[ plane ][ x4 + k ]
     }
     for ( k = 0; k < h4; k++ ) {
          if ( y4 + k < maxY4 )
              left |= LeftLevelContext[ plane ][ y4 + k ]
     }
     top = Min( top, 4 )
     left = Min( left, 4 )
     if ( fsc_mode && enable_fsc ) {
          ctx = TXB_SKIP_CONTEXTS - 1
     } else if ( bw == w && bh == h ) {
          ctx = 0
     } else {
          ctx = (top + left + 3) >> 1
     }
 } else {
     above = 0
     left = 0
     for ( i = 0; i < w4; i++ ) {
          if ( x4 + i < maxX4 ) {
              above |= AboveLevelContext[ plane ][ x4 + i ]
              above |= AboveDcContext[ plane ][ x4 + i ]
          }
     }
     for ( i = 0; i < h4; i++ ) {
          if ( y4 + i < maxY4 ) {
              left |= LeftLevelContext[ plane ][ y4 + i ]
              left |= LeftDcContext[ plane ][ y4 + i ]
          }
     }
     ctx = ( above != 0 ) + ( left != 0 )
     if ( plane == 2 ) {
          if ( bw * bh > w * h )
              ctx += 3
          if ( EobU != 0 )
              ctx += 6
     } else {
          ctx += 6
     }
 }


If plane is equal to 2, the cdf is given by TileVTxbSkipCdf[ ctx ].

Otherwise (plane is equal to 0 or 1), the cdf is given by TileTxbSkipCdf[ is_inter || fsc_mode ][ txSzCtx ]
[ ctx ].

cctx_type: the cdf is given by TileCctxTypeCdf.

eob_pt_16: the cdf is given by TileEobPt16Cdf[ eobCtx ].

eob_pt_32: the cdf is given by TileEobPt32Cdf[ eobCtx ].



AV2 Specification                                                                              Page 599 of 1169
eob_pt_64: the cdf is given by TileEobPt64Cdf[ eobCtx ].

eob_pt_128: the cdf is given by TileEobPt128Cdf[ eobCtx ].

eob_pt_256: the cdf is given by TileEobPt256Cdf[ eobCtx ].

eob_pt_512: the cdf is given by TileEobPt512Cdf[ eobCtx ].

eob_pt_1024: the cdf is given by TileEobPt1024Cdf[ eobCtx ].

eob_extra: the cdf is given by TileEobExtraCdf.

coeff_base: the variables ctx, lfCtx, hfCtx are computed as follows:

 adjTxSz = Adjusted_Tx_Size[ txSz ]
 width = Tx_Width[ adjTxSz ]
 height = Tx_Height[ adjTxSz ]
 mag = 0
 num = SIG_REF_DIFF_OFFSET_NUM
 if (plane > 0) {
     num = txClass == TX_CLASS_2D ? 3 : 2
 }
 for ( idx = 0; idx < num; idx++ ) {
     refRow = row + Sig_Ref_Diff_Offset[ txClass ][ idx ][ 0 ]
     refCol = col + Sig_Ref_Diff_Offset[ txClass ][ idx ][ 1 ]
     magLimit = ( isLf && (txClass == TX_CLASS_2D || idx < 2) &&
                   !(isHidden && c == 0) ) ? 5 : 3
     if (refRow < height && refCol < width ) {
          mag += Min( Level[ refRow ][ refCol ], magLimit )
     }
 }
 ctx = ( mag + 1 ) >> 1
 if (plane > 0) {
   ctx2 = Min( ctx, 3 )
   if (txClass != TX_CLASS_2D) {
       uvCtx = ctx2 + LF_SIG_COEF_CONTEXTS_2D_UV
   } else {
       uvCtx = (plane == 1) ? ctx2 : ctx2 + 4
   }
 } else if (isLf) {
     if (txClass == TX_CLASS_2D) {
          if (c == 0) {
              lfCtx = Min(ctx, 8)
          } else if (row + col < 2) {
              lfCtx = Min(ctx, 6) + 9
          } else {
              lfCtx = Min(ctx, 4) + 16
          }
     } else {
          idx = txClass == TX_CLASS_HORIZ ? col : row
          if (idx == 0) {
              lfCtx = LF_SIG_COEF_CONTEXTS_2D + Min(ctx, 6)
          } else {
              lfCtx = LF_SIG_COEF_CONTEXTS_2D + 7 + Min(ctx, 4)
          }
     }
 } else {
     ctx2 = Min( ctx, 4 )
     if ( txClass == TX_CLASS_2D ) {
          if (row + col < 6) {
              hfCtx = ctx2
          } else if (row + col < 8) {
              hfCtx = ctx2 + 5
          } else {
              hfCtx = ctx2 + 10
          }




AV2 Specification                                                      Page 600 of 1169
      } else {
          hfCtx = ctx2 + 15
      }
 }


If isHidden is equal to 1 and c is equal to 0, the cdf is given by TileCoeffBasePhCdf[ Min(ctx,4) ].

Otherwise, if plane is not equal to 0 and isLf is equal to 1, the cdf is given by
TileCoeffBaseLfUvCdf[ uvCtx ].

Otherwise, if plane is not equal to 0, the cdf is given by TileCoeffBaseUvCdf[ uvCtx ].

Otherwise, if isLf is equal to 1, the cdf is given by TileCoeffBaseLfCdf[ txSzCtx ][ lfCtx ][ (tcqState >> 1)
& 1 ].

Otherwise, the cdf is given by TileCoeffBaseCdf[ txSzCtx ][ hfCtx ][ (tcqState >> 1) & 1 ].

coeff_base_eob: the variable ctx is computed as follows:

 adjTxSz = Adjusted_Tx_Size[ txSz ]
 bwl = Tx_Width_Log2[ adjTxSz ]
 height = Tx_Height[ adjTxSz ]
 if (c == 0) {
     ctx = SIG_COEF_CONTEXTS_EOB - 4
 } else if (c <= (height << bwl) / 8) {
     ctx = SIG_COEF_CONTEXTS_EOB - 3
 } else if (c <= (height << bwl) / 4) {
     ctx = SIG_COEF_CONTEXTS_EOB - 2
 } else {
     ctx = SIG_COEF_CONTEXTS_EOB - 1
 }


If plane is not equal to 0 and isLf is equal to 1, the cdf is given by TileCoeffBaseLfEobUvCdf[ ctx ].

Otherwise, if plane is not equal to 0, the cdf is given by TileCoeffBaseEobUvCdf[ ctx ].

Otherwise, if isLf is equal to 1, the cdf is given by TileCoeffBaseLfEobCdf[ txSzCtx ][ ctx ].

Otherwise (plane is equal to 0 and isLf is equal to 0), the cdf is given by TileCoeffBaseEobCdf[ txSzCtx ]
[ ctx ].

coeff_base_bob: the cdf is given by TileCoeffBaseBobCdf[ Min(TX_16X16,txSzCtx) ][ctx] where ctx is
computed as follows:

 if ( bob <= (segEob>>3) ) {
     ctx = 0
 } else if ( bob <= (segEob>>2) ) {
     ctx = 1
 } else {
     ctx = 2
 }




AV2 Specification                                                                                Page 601 of 1169
coeff_base_idtx: the cdf is given by TileCoeffBaseIdtxCdf[ Min(TX_16X16,txSzCtx) ][ mag ] where mag is
computed as follows:

 mag = 0
 if (col > 0) mag += Min( 3, Level[ row ][ col - 1 ] )
 if (row > 0) mag += Min( 3, Level[ row - 1 ][ col ] )


coeff_br_idtx: the cdf is given by TileCoeffBrIdtxCdf[ Min(TX_16X16,txSzCtx) ][ mag ] where mag is
computed as follows:

 mag = 0
 if (col > 0) mag += Min( MAX_BASE_BR_RANGE - 1, Level[ row ][ col - 1 ] )
 if (row > 0) mag += Min( MAX_BASE_BR_RANGE - 1, Level[ row - 1 ][ col ] )
 mag = Min(mag, 6)


idtx_sign: the cdf is given by TileIdtxSignCdf[ Min(TX_16X16,txSzCtx) ][ ctx ] where ctx is computed as
follows:

 adjTxSz = Adjusted_Tx_Size[ txSz ]
 txw = Tx_Width[ adjTxSz ]
 signc = 0
 if (col > 0) signc += QuantSign[ row * txw + col - 1 ]
 if (row > 0) signc += QuantSign[ (row - 1) * txw + col ]
 if (col > 0 && row > 0) signc += QuantSign[ (row - 1) * txw + col - 1 ]
 if (signc > 2) ctx = 5
 else if (signc < -2) ctx = 6
 else if (signc > 0) ctx = 1
 else if (signc < 0) ctx = 2
 else ctx = 0
 if ( Level[ row ][ col ] > COEFF_BASE_RANGE && ctx != 0 ) {
     ctx += 2
 }


dc_sign: the variable ctx is computed as follows:

 maxX4 = MiCols
 maxY4 = MiRows
 dcSign = 0
 for ( k = 0; k < w4; k++ ) {
     if ( x4 + k < maxX4 ) {
         sign = AboveDcContext[ plane ][ x4 + k ]
         if ( sign == 1 ) {
             dcSign--
         } else if ( sign == 2 ) {
             dcSign++
         }
     }
 }
 for ( k = 0; k < h4; k++ ) {
     if ( y4 + k < maxY4 ) {
         sign = LeftDcContext[ plane ][ y4 + k ]
         if ( sign == 1 ) {
             dcSign--
         } else if ( sign == 2 ) {
             dcSign++
         }
     }
 }
 if ( dcSign < 0 ) {
     ctx = 1
 } else if ( dcSign > 0 ) {




AV2 Specification                                                                          Page 602 of 1169
     ctx = 2
 } else {
     ctx = 0
 }


The cdf is given by TileDcSignCdf[ ptype ][ isHidden ][ ctx ].

dc_sign_horz_vert: The cdf is given by TileDcSignCdf[ ptype ][ isHidden ][ 0 ].

coeff_br: the variables mag and ctx are computed as follows:

 adjTxSz = Adjusted_Tx_Size[ txSz ]
 bwl = Tx_Width_Log2[ adjTxSz ]
 txw = Tx_Width[ adjTxSz ]
 txh = Tx_Height[ adjTxSz ]
 row = pos >> bwl
 col = pos - (row << bwl)

 mag = 0

 txType = compute_tx_type( plane, txSz, x4, y4 )
 txClass = get_tx_class( txType )
 num = 3
 if ( txClass != TX_CLASS_2D && plane > 0 ) {
     num = 2
 }
 for ( idx = 0; idx < num; idx++ ) {
     refRow = row + Mag_Ref_Offset_With_Tx_Class[ txClass ][ idx ][ 0 ]
     refCol = col + Mag_Ref_Offset_With_Tx_Class[ txClass ][ idx ][ 1 ]
     if ( refRow < txh &&
          refCol < txw ) {
         mag += Min( Level[ refRow ][ refCol ], MAX_BASE_BR_RANGE - 1 )
     }
 }

 mag = Min( ( mag + 1 ) >> 1, 6 )
 if ( plane > 0 ) {
     ctx = Min(mag, 3)
 } else if ( pos == 0 ) {
     if (txClass != 0) {
          ctx = mag + 7
     } else {
          ctx = mag
     }
 } else {
     if (isLf ) {
          ctx = mag + 7
     } else {
          ctx = mag
     }
 }


where Mag_Ref_Offset_With_Tx_Class is defined as:

 Mag_Ref_Offset_With_Tx_Class[ 3 ][ 3 ][ 2 ] = {
   { { 0, 1 }, { 1, 0 }, { 1, 1 } },
   { { 0, 1 }, { 1, 0 }, { 0, 2 } },
   { { 0, 1 }, { 1, 0 }, { 2, 0 } }
 }




AV2 Specification                                                                 Page 603 of 1169
and get_tx_class is defined as:

 get_tx_class( txType ) {
     if ( ( txType == V_DCT ) ||
          ( txType == V_ADST ) ||
          ( txType == V_FLIPADST ) ) {
         return TX_CLASS_VERT
     } else if ( ( txType == H_DCT ) ||
                 ( txType == H_ADST ) ||
                 ( txType == H_FLIPADST ) ) {
         return TX_CLASS_HORIZ
     } else
         return TX_CLASS_2D
 }


If plane is not equal to 0, the cdf is given by TileCoeffBrUvCdf[ ctx ].

Otherwise, if isLf is equal to 1, the cdf is given by TileCoeffBrLfCdf[ ctx ].

Otherwise, the cdf is given by TileCoeffBrCdf[ ctx ].

has_palette_y: the cdf is given by TilePaletteYModeCdf.

palette_size_y_minus_2: the cdf is given by TilePaletteYSizeCdf.

palette_color_idx_y: the cdf depends on PaletteSizeY, as specified in Table 8.1:

                                  Table 8.1: Values for palette_color_idx_y

              PaletteSizeY                                                 cdf

                      2                                      TilePaletteSize2YColorCdf[ ctx ]

                      3                                      TilePaletteSize3YColorCdf[ ctx ]

                      4                                      TilePaletteSize4YColorCdf[ ctx ]

                      5                                      TilePaletteSize5YColorCdf[ ctx ]

                      6                                      TilePaletteSize6YColorCdf[ ctx ]

                      7                                      TilePaletteSize7YColorCdf[ ctx ]

                      8                                      TilePaletteSize8YColorCdf[ ctx ]


where ctx is computed as follows:

 ctx = Palette_Color_Context[ ColorContextHash ]


identity_row_y: the cdf is given by TileIdentityRowYCdf[ prevIdentityRow ].

delta_q_abs: the cdf is given by TileDeltaQCdf.

intra_tx_type: the cdf depends on the variable set, as specified in Table 8.2:

                                     Table 8.2: Values for intra_tx_type

                    set                                                  cdf

          TX_SET_WIDE_64                             TileIntraTxTypeLongCdf[ Tx_Size_Sqr[ txSz ] ]




AV2 Specification                                                                                    Page 604 of 1169
          TX_SET_WIDE_32                             TileIntraTxTypeLongCdf[ Tx_Size_Sqr[ txSz ] ]

          TX_SET_HIGH_64                             TileIntraTxTypeLongCdf[ Tx_Size_Sqr[ txSz ] ]

          TX_SET_HIGH_32                             TileIntraTxTypeLongCdf[ Tx_Size_Sqr[ txSz ] ]

          TX_SET_INTRA_1                             TileIntraTxTypeSet1Cdf[ Tx_Size_Sqr[ txSz ] ]

          TX_SET_INTRA_2                             TileIntraTxTypeSet2Cdf[ Tx_Size_Sqr[ txSz ] ]


is_long_side_dct: the cdf is given by TileIsLongSideDctCdf[is_inter].

inter_tx_type: the variables ctx and sqrSz are computed as follows:

 bwl = Min( Tx_Width_Log2[ txSz ], 5)
 eoby = (eob - 1) >> bwl
 eobx = (eob - 1) - (eoby << bwl)
 diag = eobx + eoby
 ctx = 0
 if (diag < 2) {
     ctx = 1
 } else if (diag > (Min(Tx_Width[txSz], 32) + Min(Tx_Height[txSz], 32) - 4)) {
     ctx = 2
 }
 sqrSz = Tx_Size_Sqr[ txSz ]


the cdf depends on the variable set, as specified in Table 8.3:

                        Table 8.3: CDF selection for inter_tx_type based on transform set

                          set                                                    cdf

                    TX_SET_WIDE_64                              TileInterTxTypeLongCdf[ ctx ][ sqrSz ]

                    TX_SET_WIDE_32                              TileInterTxTypeLongCdf[ ctx ][ sqrSz ]

                    TX_SET_HIGH_64                              TileInterTxTypeLongCdf[ ctx ][ sqrSz ]

                    TX_SET_HIGH_32                              TileInterTxTypeLongCdf[ ctx ][ sqrSz ]

                    TX_SET_INTER_1                              TileInterTxTypeSet1Cdf[ ctx ][ sqrSz ]

                    TX_SET_INTER_2                                  TileInterTxTypeSet2Cdf[ ctx ]

                    TX_SET_DCT_IDTX                             TileInterTxTypeSet3Cdf[ ctx ][ sqrSz ]

              TX_SET_DCT_IDTX_IDDCT                             TileInterTxTypeSet4Cdf[ ctx ][ sqrSz ]


inter_tx_type_offset: the variable ctx is computed as follows:

 bwl = Min( Tx_Width_Log2[ txSz ], 5)
 eoby = (eob - 1) >> bwl
 eobx = (eob - 1) - (eoby << bwl)
 diag = eobx + eoby
 ctx = 0
 if (diag < 2) {
     ctx = 1
 } else if (diag > (Tx_Width[txSz] + Tx_Height[txSz] - 4)) {
     ctx = 2
 }




AV2 Specification                                                                                        Page 605 of 1169
The cdf is given as follows:

  • If set is equal to TX_SET_INTER_1 and inter_tx_type is equal to 0, the cdf is given by
    TileInterTxTypeIndexSet1Cdf[ ctx ].
  • If set is equal to TX_SET_INTER_1 and inter_tx_type is equal to 1, the cdf is given by
    TileInterTxTypeOffsetSet1Cdf[ ctx ].
  • If set is equal to TX_SET_INTER_2 and inter_tx_type is equal to 0, the cdf is given by
    TileInterTxTypeIndexSet2Cdf[ ctx ].
  • If set is equal to TX_SET_INTER_2 and inter_tx_type is equal to 1, the cdf is given by
    TileInterTxTypeOffsetSet2Cdf[ ctx ].

comp_group_idx: The cdf is given by TileCompGroupIdxCdf[ ctx ], where ctx is computed as follows:

 bckOrderHint = OrderHints[ RefFrame[ 0 ] ]
 fwdOrderHint = OrderHints[ RefFrame[ 1 ] ]
 bck = Abs(get_relative_dist( OrderHint, fwdOrderHint ))
 fwd = Abs(get_relative_dist( bckOrderHint, OrderHint ))
 offset = (fwd == bck)
 ctxs[ 0 ] = 0
 ctxs[ 1 ] = 0
 for( n = 0; n < NNumBuf; n++ ) {
     if ( !NSingle[n] )
         ctxs[ n ] = CompGroupIdxs[ NPosBuf[n][0] ][ NPosBuf[n][1] ]
     else if ( NRefFrame[ n ][ 0 ] == FurthestFuture )
         ctxs[ n ] = 2
 }
 ctx0 = ctxs[ 0 ]
 ctx1 = ctxs[ 1 ]
 ctx = ctx1 + ctx0 + ( Min(ctx1,ctx0) > 0 ) + offset * 6


compound_type: The cdf is given by TileCompoundTypeCdf.

inter_intra: The cdf is given by TileInterIntraCdf[ ctx ], where ctx is computed as follows:

 ctx = Size_Group[ MiSize ]


warp_inter_intra: The cdf is given by TileWarpInterIntraCdf[ ctx ], where ctx is computed as follows:

 ctx = Size_Group[ MiSize ]


interintra_mode: The cdf is given by TileInterIntraModeCdf[ ctx ], where ctx is computed as follows:

 ctx = Size_Group[ MiSize ]


wedge_quad: The cdf is given by TileWedgeQuadCdf.

wedge_angle: The cdf is given by TileWedgeAngleCdf[wedge_quad].

wedge_dist1: The cdf is given by TileWedgeDist1Cdf.

wedge_dist2: The cdf is given by TileWedgeDist2Cdf.

wedge_interintra: The cdf is given by TileWedgeInterIntraCdf.


AV2 Specification                                                                              Page 606 of 1169
warp_delta_precision: The cdf is given by TileWarpDeltaPrecisionCdf[ MiSize ].

warp_delta_param_low: The cdf is given by TileWarpDeltaParamLowCdf[ idx==3 || idx==4 ].

warp_delta_param_high: The cdf is given by TileWarpDeltaParamHighCdf[ idx==3 || idx==4 ].

warp_delta_param_sign: The cdf is given by TileWarpDeltaParamSignCdf.

ccso_blk: The cdf is given by TileCcsoBlkCdf[plane][ctx], where ctx is computed as follows:

 if ( MiCol - blkW4 >= MiColStart ) {
     ctx = 2 * CcsoBlks[ plane ][ MiRow >> shiftRow ]
                                [ (MiCol - blkW4) >> shiftCol ]
 } else {
     ctx = 0
 }


cfl_index: The cdf is given by TileCflIndexCdf.

cfl_alpha_signs: The cdf is given by TileCflSignCdf.

cfl_alpha_u: The cdf is given by TileCflAlphaCdf[ ctx ], where ctx is obtained from the following table:

                                 Table 8.4: Context selection for cfl_alpha_u

                                    cfl_alpha_signs                                            ctx

                                          0                                                    N/A

                                          1                                                    N/A

                                          2                                                     0

                                          3                                                     1

                                          4                                                     2

                                          5                                                     3

                                          6                                                     4

                                          7                                                     5



  NOTE:       N/A is used to indicate that no context is needed as the sign is zero and no value is decoded.


or computed as follows:

 ctx = (signU - 1) * 3 + signV



  NOTE: As shown in the previous table, the variable ctx produced by this calculation will be equal to
  cfl_alpha_signs - 2.


cfl_alpha_v: The cdf is given by TileCflAlphaCdf[ ctx ], where ctx is obtained from the following Table
8.5:

                    Table 8.5: Context calculation for cfl_alpha_v based on cfl_alpha_signs




AV2 Specification                                                                               Page 607 of 1169
                                    cfl_alpha_signs                                            ctx

                                          0                                                     0

                                          1                                                     3

                                          2                                                    N/A

                                          3                                                     1

                                          4                                                     4

                                          5                                                    N/A

                                          6                                                     2

                                          7                                                     5



  NOTE:       N/A is used to indicate that no context is needed as the sign is zero and no value is decoded.


or computed as follows:

 ctx = (signV - 1) * 3 + signU


cfl_mhccp: The cdf is given by TileCflMhccpCdf.

cfl_mh_dir: The cdf is given by TileCflMhDirCdf[ Size_Group[ MiSize ] ].

use_wiener_ns: The cdf is given by TileUseWienerNsCdf.

use_pc_wiener: The cdf is given by TileUsePcWienerCdf.

flex_restoration_type: The cdf is given by TileFlexRestorationTypeCdf[ tool ][ plane ].

wiener_ns_base: The cdf is given by TileWienerNsBaseCdf.

wiener_ns_length: The cdf is given by TileWienerNsLengthCdf[ Min(plane, 1) ].

wiener_ns_uv_sym: The cdf is given by TileWienerNsUvSymCdf.

                                                                                    ↑ Back to Table of Contents




AV2 Specification                                                                               Page 608 of 1169
```
