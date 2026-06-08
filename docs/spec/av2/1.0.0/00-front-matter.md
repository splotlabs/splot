# AV2 v1.0.0 — Front matter

<!-- Verbatim mirror of the AOM AV2 v1.0.0 specification (© Alliance for Open Media). The PDF is normative; this is a faithful `pdftotext -layout` copy. See [./README.md](./README.md) and [./index.md](./index.md). Do not hand-edit: regenerate via scripts/spec/regenerate-av2-spec.sh. -->

```text
AV2 Bitstream & Decoding
Process Specification
Final Deliverable, 28 May 2026
Version:
   v1.0.0
Issue Tracking:
    GitHub

Copyright 2026, Alliance for Open Media

Licensing information is available at https://www.aomedia.org/license/

The MATERIALS ARE PROVIDED “AS IS.” The Alliance for Open Media, its members, and its contributors
expressly disclaim any warranties (express, implied, or otherwise), including implied warranties of
merchantability, non-infringement, fitness for a particular purpose, or title, related to the materials. The
entire risk as to implementing or otherwise using the materials is assumed by the implementer and user.
IN NO EVENT WILL THE ALLIANCE FOR OPEN MEDIA, ITS MEMBERS, OR CONTRIBUTORS BE
LIABLE TO ANY OTHER PARTY FOR LOST PROFITS OR ANY FORM OF INDIRECT, SPECIAL,
INCIDENTAL, OR CONSEQUENTIAL DAMAGES OF ANY CHARACTER FROM ANY CAUSES OF ACTION
OF ANY KIND WITH RESPECT TO THIS DELIVERABLE OR ITS GOVERNING AGREEMENT, WHETHER
BASED ON BREACH OF CONTRACT, TORT (INCLUDING NEGLIGENCE), OR OTHERWISE, AND
WHETHER OR NOT THE OTHER PARTY HAS BEEN ADVISED OF THE POSSIBILITY OF SUCH
DAMAGE.



Abstract
This document defines the bitstream format and decoding process for the Alliance for Open Media Video
2 (AV2) codec.




AV2 Specification                                                                               Page 1 of 1169
    Table of Contents                                                                                                                             Pages


     1 Scope . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                            17. . . . . . .

     2 Terms and definitions . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .18. . . . . . .

     3 Symbols . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                          29. . . . . . .

     4 Conventions . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .41
                                                                                                                                                         .......

           4.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                          41. . . . . . .

           4.2 Arithmetic operators . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                   41. . . . . .

           4.3 Ternary operator . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                     41. . . . . . .

           4.4 Logical operators . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .42
                                                                                                                                                       .......

           4.5 Relational operators . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                   42. . . . . . .

           4.6 Bitwise operators . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .42
                                                                                                                                                       .......

           4.7 Assignment . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                       42. . . . . . .

           4.8 Mathematical functions . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .42
                                                                                                                                                  .......

           4.9 Method of describing bitstream syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .44
                                                                                                                                         .......

           4.10 Functions . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .46
                                                                                                                                                          .......

           4.11 Descriptors . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                       46. . . . . . .

                 4.11.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                       46. . . . . . .

                 4.11.2 f(n) . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .46
                                                                                                                                                             .......

                 4.11.3 uvlc() . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                          46. . . . . . .

                 4.11.4 svlc() . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                          47. . . . . . .

                 4.11.5 le(n) . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                           47. . . . . .

                 4.11.6 leb128() . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                        47. . . . . .

                 4.11.7 su(n) . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .48
                                                                                                                                                            .......

                 4.11.8 ns(n) . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .49
                                                                                                                                                            .......

                 4.11.9 tu(mx) . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                        49. . . . . . .

                 4.11.10 rg(n) . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .50
                                                                                                                                                           .......

                 4.11.11 L(n) . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                         50. . . . . . .

                 4.11.12 S() . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                          50. . . . . . .

                 4.11.13 NS(n) . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .50
                                                                                                                                                         .......

     5 Syntax structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                    52. . . . . .

           5.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                          52. . . . . . .




AV2 Specification                                                                                                                            Page 2 of 1169
          5.2 OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                      52. . . . . . .

                5.2.1 General OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .52. . . . . . .

                5.2.2 OBU header syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                               54. . . . . . .

                5.2.3 Trailing bits syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                  54. . . . . . .

                5.2.4 Byte alignment syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .55
                                                                                                                                                .......

          5.3 Reserved OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                               55. . . . . . .

          5.4 Sequence header OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .55
                                                                                                                                           .......

                5.4.1 General sequence header OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                  55. . . . . . .

                5.4.2 Sequence tile config syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .59
                                                                                                                                              .......

                5.4.3 Sequence partition config syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .59. . . . . . .

                5.4.4 Sequence segment config syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .60
                                                                                                                                         .......

                5.4.5 Sequence intra config syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .60
                                                                                                                                             .......

                5.4.6 Sequence inter config syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .60
                                                                                                                                             .......

                5.4.7 Sequence screen content config syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .63
                                                                                                                                      .......

                5.4.8 Sequence transform quant entropy config syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . .64
                                                                                                                               .......

                5.4.9 Segment information syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          66. . . . . . .

                5.4.10 Sequence filter config syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .66. . . . . . .

                5.4.11 User defined QM syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           67. . . . . . .

                5.4.12 Timing info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .69
                                                                                                                                                  .......

                5.4.13 Sequence decoder model info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                   69. . . . . . .

          5.5 Temporal delimiter OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .69. . . . . . .

          5.6 Multi Stream Decoder Operation OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .69. . . . . . .

          5.7 Multi frame header OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         70. . . . . . .

          5.8 Layer config record OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          71. . . . . .

                5.8.1 LCR global info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .71
                                                                                                                                                 .......

                5.8.2 LCR local info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .72
                                                                                                                                                  .......

                5.8.3 LCR aggregate info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .72. . . . . . .

                5.8.4 LCR sequence profile tier level information syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . .73
                                                                                                                                 .......

                5.8.5 LCR global payload syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .73
                                                                                                                                              .......

                5.8.6 LCR xlayer info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .73
                                                                                                                                                 .......

                5.8.7 LCR rep info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .74
                                                                                                                                                  .......

                5.8.8 LCR embedded layer info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .74
                                                                                                                                         .......




AV2 Specification                                                                                                                       Page 3 of 1169
               5.8.9 LCR xlayer color info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           75. . . . . . .

          5.9 Atlas segment info OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .75
                                                                                                                                            .......

               5.9.1 Atlas label segment info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .76
                                                                                                                                           .......

               5.9.2 Atlas enhanced atlas info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         76. . . . . .

               5.9.3 Atlas multistream info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          78. . . . . . .

               5.9.4 Atlas multistream with alpha info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .78. . . . . . .

               5.9.5 Atlas basic info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .79
                                                                                                                                                 .......

          5.10 Operating point set OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         80. . . . . . .

          5.11 Operating point payload syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .80
                                                                                                                                            .......

               5.11.1 Operating point set aggregate info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                 82. . . . . . .

               5.11.2 Operating point set sequence profile tier level information syntax . . . . . . . . . . . . . . .
                                                                                                                    82. . . . . . .

               5.11.3 Operating point set decoder model info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .82
                                                                                                                                .......

               5.11.4 Operating point set color info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     82. . . . . . .

               5.11.5 Operating point set mlayer info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .82
                                                                                                                                     .......

          5.12 Buffer removal timing OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       83. . . . . . .

          5.13 Quantizer Matrix OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .83
                                                                                                                                           .......

          5.14 Film grain OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                              84. . . . . . .

          5.15 Content interpretation OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .85
                                                                                                                                         .......

          5.16 Padding OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                               86. . . . . . .

          5.17 Metadata OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                              87. . . . . .

               5.17.1 Metadata unit syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .87
                                                                                                                                               .......

               5.17.2 Metadata short OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .88. . . . . . .

               5.17.3 Metadata group OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .88
                                                                                                                                        .......

               5.17.4 Metadata ITUT T35 syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                        90. . . . . . .

               5.17.5 Metadata high dynamic range content light level syntax . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                        90. . . . . .

               5.17.6 Metadata high dynamic range mastering display color volume syntax . . . . . . . . . . . .90. . . . . . .

               5.17.7 Metadata timecode syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .90
                                                                                                                                           .......

               5.17.8 Metadata banding hints syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .91
                                                                                                                                        .......

               5.17.9 Metadata ICC profile syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         92. . . . . . .

               5.17.10 Metadata scan type syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                        92. . . . . . .

               5.17.11 Metadata temporal point info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                  92. . . . . . .

               5.17.12 Metadata decoded frame hash syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .93
                                                                                                                                  .......




AV2 Specification                                                                                                                 Page 4 of 1169
               5.17.13 Metadata user data unregistered syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .93. . . . . . .

          5.18 Frame header syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .93
                                                                                                                                                 .......

               5.18.1 General frame header syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       93. . . . . . .

               5.18.2 Frame header info syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          94. . . . . . .

               5.18.3 Frame configuration structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     113 .......

               5.18.4 Frame size structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           115........

               5.18.5 Filtering structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                              116.......

               5.18.6 Quantization structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          118 .......

               5.18.7 Segmentation and tiling structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                   120 .......

               5.18.8 Transform and coding mode structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                              135........

               5.18.9 Global motion structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         137 .......

               5.18.10 Film grain structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           141 .......

          5.19 Tile group OBU syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .144
                                                                                                                                               ........

          5.20 Tile group payload syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                            145.......

               5.20.1 General tile group payload syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                    145 .......

               5.20.2 Tile-level structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                              146 .......

               5.20.3 Partition structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                              148.......

               5.20.4 Block decoding structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                        161 .......

               5.20.5 Mode information structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      169 .......

               5.20.6 Transform and quantization structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                182 .......

               5.20.7 Motion vector and prediction structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                186 .......

               5.20.8 Coding tools structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          235 .......

               5.20.9 Helper functions . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                              243........

               5.20.10 Filtering structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                            244 .......

     6 Syntax structures semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         255 .......

          6.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                       255........

          6.2 OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                 255........

               6.2.1 General OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         255 .......

               6.2.2 OBU header semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         255........

               6.2.3 Trailing bits semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                             258 .......

               6.2.4 Byte alignment semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         258........

          6.3 Reserved OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           258. . . . . . .




AV2 Specification                                                                                                                     Page 5 of 1169
          6.4 Sequence header OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     258.......

               6.4.1 General sequence header OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                             258 .......

               6.4.2 Sequence tile config semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       264.......

               6.4.3 Sequence partition config semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                   264 .......

               6.4.4 Sequence segment config semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                 265 .......

               6.4.5 Sequence intra config semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     266 .......

               6.4.6 Sequence inter config semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     266 .......

               6.4.7 Sequence screen content config semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                              270 .......

               6.4.8 Sequence transform quant entropy config semantics . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                       270 .......

               6.4.9 Segment information semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                    271........

               6.4.10 Sequence filter config semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     271 .......

               6.4.11 User defined QM semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      273 .......

               6.4.12 Timing info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           273.......

               6.4.13 Sequence decoder model info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                              274. . . . . . .

          6.5 Temporal delimiter OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     274 .......

          6.6 Multi Stream Decoder Operation OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                           274 .......

          6.7 Multi frame header OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                    276........

          6.8 Layer config record OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     276........

               6.8.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                    277........

               6.8.2 LCR global info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         278 .......

               6.8.3 LCR local info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           280.......

               6.8.4 LCR aggregate info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      281 .......

               6.8.5 LCR sequence profile tier level information semantics . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                         281 .......

               6.8.6 LCR global payload semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      282 .......

               6.8.7 LCR xlayer info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         282 .......

               6.8.8 LCR rep info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           284........

               6.8.9 LCR embedded layer info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                 284 .......

               6.8.10 LCR xlayer color info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     287........

          6.9 Atlas segment info OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      288........

               6.9.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                    288........

               6.9.2 Atlas label segment info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     290.......

               6.9.3 Atlas enhanced atlas info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                    290........




AV2 Specification                                                                                                                  Page 6 of 1169
               6.9.4 Atlas multistream info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      292 .......

               6.9.5 Atlas multistream with alpha info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                               292 .......

               6.9.6 Atlas basic info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           292........

          6.10 Operating point set OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                    293........

               6.10.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                   293........

               6.10.2 Operating point set OBU syntax elements . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .294
                                                                                                                                ........

               6.10.3 Operating point set aggregate info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .297
                                                                                                                               ........

               6.10.4 Operating point set sequence profile tier level information semantics . . . . . . . . . . . .
                                                                                                                297 .......

               6.10.5 Operating point set decoder model info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                         297 .......

               6.10.6 Operating point set color info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                 298 .......

               6.10.7 Operating point set mlayer info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                               300........

          6.11 Buffer removal timing OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                  301........

          6.12 Quantizer Matrix OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     301........

          6.13 Film grain OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          302 .......

          6.14 Content interpretation OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                  303 .......

          6.15 Padding OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          305........

          6.16 Metadata OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         305........

               6.16.1 Metadata unit semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                        305 .......

               6.16.2 Metadata short OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                   305 .......

               6.16.3 Metadata group OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                  306.......

               6.16.4 Metadata ITUT T35 semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .309
                                                                                                                                      ........

               6.16.5 Metadata high dynamic range content light level semantics . . . . . . . . . . . . . . . . . . .
                                                                                                                   309........

               6.16.6 Metadata high dynamic range mastering display color volume semantics . . . . . . . . .
                                                                                                         310 .......

               6.16.7 Metadata timecode semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                    312 .......

               6.16.8 Metadata banding hints semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                  313........

               6.16.9 Metadata ICC profile semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                    315........

               6.16.10 Metadata scan type semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                   315........

               6.16.11 Metadata temporal point info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                              316 .......

               6.16.12 Metadata user data unregistered semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                           317 .......

               6.16.13 Metadata decoded frame hash semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                           317 .......

          6.17 Frame header OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      319 .......

               6.17.1 General frame header semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                   319 .......




AV2 Specification                                                                                                                 Page 7 of 1169
                6.17.2 Frame header info semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      319........

                6.17.3 Frame configuration structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      334 .......

                6.17.4 Frame size structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                            335........

                6.17.5 Filtering structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                               337.......

                6.17.6 Quantization structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           337 .......

                6.17.7 Segmentation and tiling structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                    339 .......

                6.17.8 Transform and coding mode structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                               344........

                6.17.9 Global motion structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          345 .......

                6.17.10 Film grain structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                            346 .......

          6.18 Tile group OBU semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          349 .......

          6.19 Tile group payload semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         349........

                6.19.1 General tile group payload semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                  349 .......

                6.19.2 Tile-level structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                               350 .......

                6.19.3 Partition structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                               351.......

                6.19.4 Block decoding structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         354 .......

                6.19.5 Mode information structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       355 .......

                6.19.6 Transform and quantization structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                 358 .......

                6.19.7 Motion vector and prediction structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                 360 .......

                6.19.8 Coding tools structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           372 .......

                6.19.9 Filtering structures . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                               373.......

     7 Decoding process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                 375.......

          7.1 General decoding process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           375 .......

          7.2 Decode frame wrapup process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       376........

          7.3 Ordering of OBUs . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                379........

                7.3.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                     379........

                7.3.2 Coded multistream video sequence boundaries . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                           380........

                7.3.3 Coded output frame unit . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           380........

                7.3.4 Coded non-output frame unit . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                        382 .......

                7.3.5 Coded frame unit . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                               383 .......

                7.3.6 Coded extended layer unit . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          383 .......

                7.3.7 Temporal unit . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                  385 .......

                7.3.8 Availability of high level syntax OBUs . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     386 .......




AV2 Specification                                                                                                                   Page 8 of 1169
               7.3.9 Availability of long-term reference frames . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                 389........

          7.4 Random access decoding . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           390 .......

               7.4.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                    390........

               7.4.2 Random access and use of long-term reference frames . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                      391........

               7.4.3 Closed Random Access . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         391........

               7.4.4 Open Random Access . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .391
                                                                                                                                            ........

               7.4.5 Random Access Switch . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         393........

               7.4.6 Multistream Random Access . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     394 .......

          7.5 Frame end update CDF process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      395........

          7.6 Extended layer context management . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                   395........

          7.7 Get ref frames process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .397
                                                                                                                                                 ........

          7.8 Get past future cur ref lists process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         400........

          7.9 Motion field estimation process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          401 .......

               7.9.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                    401........

               7.9.2 Fill trajectory motion vector gap process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                  406........

               7.9.3 Projection process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                               407.......

               7.9.4 Get MV projection process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .411
                                                                                                                                           ........

               7.9.5 Get MV projection clamp process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                    411........

               7.9.6 Get sampled position process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       412........

               7.9.7 Get block position no constraint process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                  412 .......

               7.9.8 Check block position process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                        413. . . . . . .

          7.10 Setup TIP motion field process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         413.......

               7.10.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                   413........

               7.10.2 TIP temporal scale motion field process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                 414.......

               7.10.3 TIP fill motion field holes process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       415.......

               7.10.4 TIP block average filter motion vector process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                             415 .......

               7.10.5 Fill temporal motion vectors sample gap process . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                          416 .......

               7.10.6 Build TIP process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                              418 .......

          7.11 Motion vector context processes . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       419 .......

               7.11.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                   419........

               7.11.2 Find mode context process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       419........

               7.11.3 Scan point context process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                        420........




AV2 Specification                                                                                                                  Page 9 of 1169
               7.11.4 Scan point warp context process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                    421 .......

          7.12 Motion vector prediction processes . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     422........

               7.12.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                   422........

               7.12.2 Find MV stack process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .422
                                                                                                                                            ........

               7.12.3 Find warp samples process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      444 .......

          7.13 Prediction processes . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                               447.......

               7.13.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                   447........

               7.13.2 Intra prediction process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          447........

               7.13.3 Inter prediction process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          465........

               7.13.4 Palette prediction process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         507 .......

               7.13.5 Predict chroma from luma process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                  508.......

               7.13.6 MHCCP process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                            511 .......

               7.13.7 Derive multi param process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       512. . . . . . .

               7.13.8 Gaussian elimination process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      514........

          7.14 Reconstruction and dequantization . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     515 .......

               7.14.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                   515........

               7.14.2 Dequantization functions . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         515 .......

               7.14.3 Reconstruct process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           516........

               7.14.4 Dequantization process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         517 .......

               7.14.5 Save dequant process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          519........

               7.14.6 Get dequant process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           519........

          7.15 Inverse transform process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           520 .......

               7.15.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                   520........

               7.15.2 1D transforms . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                               520........

               7.15.3 Secondary transform process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     522........

               7.15.4 2D inverse transform process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      524.......

          7.16 Deblocking filter for TIP process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         528 .......

          7.17 Deblocking filter process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                             530 .......

               7.17.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                   530........

               7.17.2 Edge deblocking filter process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      531........

               7.17.3 Filter maximum width process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                    535........

               7.17.4 Filter size process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                               536.......




AV2 Specification                                                                                                                Page 10 of 1169
                7.17.5 Adaptive filter strength process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       536........

                7.17.6 Adaptive filter strength selection process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                  537 .......

                7.17.7 Sample filtering process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           537........

          7.18 CDEF process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                 540........

                7.18.1 CDEF block process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                            540 .......

                7.18.2 CDEF direction process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          542 .......

                7.18.3 CDEF filter process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                             544 .......

          7.19 CCSO process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                 545........

                7.19.1 Apply CCSO filter process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         546 .......

          7.20 Loop restoration process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                            548 .......

                7.20.1 Loop restore block process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         549.......

                7.20.2 Get source sample process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                        552........

                7.20.3 Non-separable Wiener filter process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                   553 .......

                7.20.4 Pixel classified Wiener filter process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      555 .......

                7.20.5 Apply GDF filter process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           557.......

          7.21 Output processes . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                560 .......

                7.21.1 Output process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .560
                                                                                                                                                  ........

                7.21.2 Intermediate output preparation process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                561.......

                7.21.3 Output successive frames process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                    562. . . . . . .

                7.21.4 Output implicit output frame process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                   562.......

                7.21.5 Flush implicit output frames process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                   563........

                7.21.6 Output frame buffers process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       564........

                7.21.7 Film grain synthesis process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                        565 .......

          7.22 Motion field motion vector storage process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                 572........

          7.23 Reference frame update process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      576 .......

     8 Parsing process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                   579 .......

          8.1 Parsing process for f(n) . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                               579 .......

          8.2 Parsing process for symbol decoder . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      579........

                8.2.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                     579........

                8.2.2 Initialization process for symbol decoder . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                   579........

                8.2.3 Boolean decoding process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         585 .......

                8.2.4 Exit process for symbol decoder . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       585........




AV2 Specification                                                                                                                  Page 11 of 1169
                8.2.5 Parsing process for read_literal . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         587 .......

                8.2.6 Symbol decoding process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         587........

          8.3 Parsing process for CDF encoded syntax elements . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                            588 .......

                8.3.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                     588........

                8.3.2 Cdf selection process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                              589 .......

     9 Additional tables . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                   609 .......

          9.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                       609........

          9.2 Conversion tables . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                 609........

          9.3 Default CDF tables . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .623
                                                                                                                                                   ........

          9.4 Quantizer matrix tables . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                              742 .......

                9.4.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                     742........

                9.4.2 Derivation process (Informative) . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       743 .......

                9.4.3 Tables . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                       743 .......

          9.5 Warp filter tables . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                   841 .......

          9.6 1d transform tables . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                845 .......

          9.7 Secondary transform tables . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           849 .......

          9.8 Loop restoration tables . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                               921........

      Annex A: Profiles, levels, and tiers . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       1101........

          A.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                     1101 ........

          A.2 Profiles . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                       1101........

          A.3 Multi-sequence configurations . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       1104 ........

          A.4 Levels . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                      1104 ........

          A.5 Decoder Conformance . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          1111........

      Annex B: Length delimited bitstream format . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                          1112 ........

          B.1 Overview . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                   1112........

          B.2 Length delimited bitstream syntax . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                    1112........

          B.3 Length delimited bitstream semantics . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                  1112 ........

      Annex C: Error resilience behavior (informative) . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                         1113........

          C.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                     1113 ........

          C.2 Definition of processable frames . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       1113........

          C.3 Recommendation for processable frames . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                              1114........

          C.4 Encoder consequences of processable frames . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                            1114 ........




AV2 Specification                                                                                                                    Page 12 of 1169
          C.5 Decoder consequences of processable frames . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                            1114 ........

      Annex D: Multistream composition process (informative) . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                1115 ........

          D.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                     1115 ........

          D.2 Chroma format determination process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                1116........

          D.3 Array initialization process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           1117........

               D.3.1 Background color determination process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                            1117........

          D.4 Spatial mapping process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          1118........

          D.5 Spatial mapping with alpha process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                   1119........

               D.5.1 Frame resampling process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     1120 ........

               D.5.2 Monochrome frame resampling process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                           1121........

      Annex E: Decoder model . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          1122 ........

          E.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                     1122 ........

          E.2 Operating point selection . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           1123 ........

          E.3 Decoder model definitions . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          1123........

          E.4 Operating modes . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                              1126........

               E.4.1 Resource availability mode . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       1126 ........

               E.4.2 Decoding schedule mode . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      1127........

               E.4.3 Establishing bitstream conformance . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                1128........

               E.4.4 When timing information is not present in the bitstream . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                     1128........

          E.5 Frame timing definitions . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           1128........

               E.5.1 Start of DFG bits arrival . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          1128 ........

               E.5.2 End of DFG bits arrival . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          1129 ........

               E.5.3 Scheduled removal times . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       1129........

               E.5.4 Removal times in decoding schedule mode . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                           1129........

               E.5.5 Removal times in resource availability mode . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                            1130 ........

               E.5.6 Frame decode timing . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         1131........

               E.5.7 Frame presentation timing . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      1131 ........

          E.6 Decoder model . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                1132........

               E.6.1 Decoder model structure . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       1132........

               E.6.2 Decoder model functions . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       1133........

               E.6.3 Decoder model error codes . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     1136........

          E.7 Bitstream conformance . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                          1136........




AV2 Specification                                                                                                                   Page 13 of 1169
               E.7.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                  1136 ........

               E.7.2 Decoder buffer delay consistency across random access points (applies to decoding
                                                                                                     1137
                                                                                                       schedule m

               E.7.3 Smoothing buffer overflow . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      1137 ........

               E.7.4 Smoothing buffer underflow . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     1137 ........

               E.7.5 Minimum decode time (applies to decoding schedule mode) . . . . . . . . . . . . . . . . . . . .
                                                                                                                1137 .......

               E.7.6 Minimum presentation interval . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                   1138........

               E.7.7 Decode deadline . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                            1138 ........

               E.7.8 Level imposed constraints . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                       1138........

               E.7.9 Decode Process constraints . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      1138........

      Annex F: Sub-bitstream extraction (informative) . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                         1139 ........

          F.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                      1139........

          F.2 Operating point usage . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                            1139........

               F.2.1 General decoder operation . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      1139 ........

               F.2.2 Multistream bitstream decoder operation . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                             1139........

               F.2.3 Singlestream bitstream decoder operation . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                             1142 .......

          F.3 Sub-bitstream extraction processes . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     1143........

               F.3.1 Operating point selection and analysis process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                            1145........

               F.3.2 Sub-bitstream extraction process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                   1149 ........

               F.3.3 Preserved OBU types . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         1150........

      Annex G: Layer composition and Atlas usage examples (informative) . . . . . . . . . . . . . . . . . .
                                                                                                        1151........

          G.1 General . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                     1151 ........

          G.2 360-degree viewport-dependent streaming with subpictures . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                   1151........

               G.2.1 Layer structure . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                              1152 ........

               G.2.2 LCR configuration . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           1153........

               G.2.3 Atlas configuration . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                            1154 ........

               G.2.4 Viewport-dependent streaming process . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                             1155 ........

               G.2.5 Benefits for 360-degree streaming . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                  1156 ........

          G.3 Subpicture composition example . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                    1158 ........

               G.3.1 LCR configuration . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           1158........

               G.3.2 Atlas configuration . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                            1159 ........

               G.3.3 Rendering and adaptive streaming . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                1159........

          G.4 Region-of-interest scalability example with encoder padding . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                     1161 ........




AV2 Specification                                                                                                                   Page 14 of 1169
                G.4.1 LCR configuration . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                            1162........

                G.4.2 Atlas configuration . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                             1162 ........

                G.4.3 Rendering scenarios . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           1163 ........

          G.5 Implementation considerations . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                      1165........

                G.5.1 Decoder requirements . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                         1165........

                G.5.2 Encoder recommendations . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     1166 ........

                G.5.3 Interoperability . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                1166 .......

      Index . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                          1167........

      Terms defined by this specification . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                     1167 ........

      References . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                                     1168........

      Normative References . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                            1168 ........

      Informative References . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .
                                                                                                                                           1169........




AV2 Specification                                                                                                                       Page 15 of 1169
Introduction
This document specifies the bitstream format and decoding process for the Alliance for Open Media
Video 2 (AV2) codec. It is intended to be read by implementers of AV2 decoders and encoders, by authors
of container and transport formats that carry AV2 bitstreams, and by authors of conformance tests.

A conforming AV2 decoder is fully specified by the normative content of § 4 Conventions, § 5 Syntax
structures, § 6 Syntax structures semantics, § 7 Decoding process, § 8 Parsing process, Annex A: Profiles,
levels, and tiers, and Annex E: Decoder model. The informative annexes describe recommended behavior
and illustrative use cases and are not required for conformance. Informative content is identified either
by an annex title ending with "(informative)", by an introductory statement, or by paragraphs beginning
with the word "Note" that are visually set apart from the surrounding text.

A first reading of this document is recommended in the following order:

 1. § 2 Terms and definitions and § 3 Symbols to establish vocabulary. Defined terms appear as links
    throughout the document, and following a link navigates to the term’s definition.
 2. § 4 Conventions to understand the mathematical operators, pseudocode style, and descriptor notation
    used in the syntax tables. Syntax element descriptors such as f(n) and L(n) are defined in § 8 Parsing
    process.
 3. § 5 Syntax structures alongside § 6 Syntax structures semantics. The syntax structures, presented as
    pseudocode, define the order in which bits are read. The semantics define the meaning of each syntax
    element and the variables it updates.
 4. § 7 Decoding process and § 8 Parsing process, which together describe how a conforming decoder
    transforms a sequence of OBUs into decoded frames.
 5. Annex A: Profiles, levels, and tiers for conformance constraints and Annex E: Decoder model for the
    decoder model.

The informative annexes may be consulted as needed: Annex C: Error resilience behavior (informative)
for decoding from non-key starting points, Annex D: Multistream composition process (informative) for
composing decoded frames from a multistream, Annex F: Sub-bitstream extraction (informative) for
extracting sub-bitstreams based on operating points, and Annex G: Layer composition and Atlas usage
examples (informative) for usage examples of the layer configuration record.




AV2 Specification                                                                            Page 16 of 1169
```
