# AV2 v1.0.0 — § 4. Conventions

<!-- Verbatim mirror of the AOM AV2 v1.0.0 specification (© Alliance for Open Media). The PDF is normative; this is a faithful `pdftotext -layout` copy. See [./README.md](./README.md) and [./index.md](./index.md). Do not hand-edit: regenerate via scripts/spec/regenerate-av2-spec.sh. -->

<a id="s-4"></a>

## § 4 Conventions

```text
§   4. Conventions
```

<a id="s-4-1"></a>

### § 4.1 General

```text
§   4.1. General
    The mathematical operators and their precedence rules used to describe this specification are similar to
    those used in the C programming language. However, the operation of integer division with truncation is
    specifically defined.

    In addition, a length 2 array used to hold a motion vector (indicated by the variable name ending with the
    letters Mv or Mvs) can be accessed using either array notation (e.g., Mv[ 0 ] and Mv[ 1 ]), or by just the
    name (e.g., Mv). The only operations defined when using the name are assignment and equality/inequality
    testing. Assignment of an array is represented using the notation A = B and is specified to mean the same
    as doing both the individual assignments A[ 0 ] = B[ 0 ] and A[ 1 ] = B[ 1 ]. Equality testing of 2 motion
    vectors is represented using the notation A == B and is specified to mean the same as (A[ 0 ] == B[ 0 ] &&
    A[ 1 ] == B[ 1 ]). Inequality testing is defined as A != B and is specified to mean the same as (A[ 0 ] !=
    B[ 0 ] || A[ 1 ] != B[ 1 ]).


    If a process specifies something happens for x = L..H, where x is a variable name and L and H are
    expressions, it means that the variable takes all integer values starting at L and going up to (and
    including) H.

    When a variable is said to be representable by a signed integer with x bits, it means that the variable is
    greater than or equal to -(1 << (x-1)), and that the variable is less than or equal to (1 << (x-1))-1.

    The key words “must”, “must not”, “required”, “shall”, “shall not”, “should”, “should not”,
    “recommended”, “may”, and “optional” in this document are to be interpreted as described in [RFC2119].

    Informative notes begin with the word “Note” and are set apart from the normative text with class="note",
    like this:


         NOTE:      This is an informative note.


```

<a id="s-4-2"></a>

### § 4.2 Arithmetic operators

```text
§   4.2. Arithmetic operators
     +           Addition

     –           Subtraction (as a binary operator) or negation (as a unary prefix operator)

     *           Multiplication

     /           Integer division with truncation of the result toward zero (for example, 7/4 and -7/-4 are truncated to 1, and -7/4 and
                 7/-4 are truncated to -1)

     a%b         Remainder from division of a by b, where both a and b are positive integers

     ÷           Floating point (arithmetical) division

     ceil(x)     The smallest integer that is greater than or equal to x

     floor(x)    The largest integer that is less than or equal to x


```

<a id="s-4-3"></a>

### § 4.3 Ternary operator

```text
§   4.3. Ternary operator
     cond ? a : b                            a if cond is true, b if cond is false




    AV2 Specification                                                                                                     Page 41 of 1169
```

<a id="s-4-4"></a>

### § 4.4 Logical operators

```text
§   4.4. Logical operators
     a && b                     Logical AND operation between a and b

     a || b                     Logical OR operation between a and b

     !                          Logical NOT operation


```

<a id="s-4-5"></a>

### § 4.5 Relational operators

```text
§   4.5. Relational operators
     >                        Greater than

     >=                       Greater than or equal to

     <                        Less than

     <=                       Less than or equal to

     ==                       Equal to

     !=                       Not equal to


```

<a id="s-4-6"></a>

### § 4.6 Bitwise operators

```text
§   4.6. Bitwise operators
     &           AND operation

     |           OR operation

     ^           XOR operation

     ~           Negation operation

     a >> b      Shift a in 2’s complement binary integer representation format to the right by b bit positions. This operator is only used
                 with b being a non-negative integer. Bits shifted into the MSBs as a result of the right shift have a value equal to the
                 MSB of a prior to the shift operation.

     a << b      Shift a in 2’s complement binary integer representation format to the left by b bit positions. This operator is only used
                 with b being a non-negative integer. Bits shifted into the LSBs as a result of the left shift have a value equal to 0.


```

<a id="s-4-7"></a>

### § 4.7 Assignment

```text
§   4.7. Assignment
     =        Assignment operator

     ++       Increment (for example, x++ is equivalent to x = x + 1). When this operator is used for an array index, the variable value
              is obtained before the auto increment operation.

     --       Decrement (for example, x-- is equivalent to x = x - 1). When this operator is used for an array index, the variable value
              is obtained before the auto decrement operation.

     +=       Addition assignment operator (for example, x += 3 corresponds to x = x + 3)

     -=       Subtraction assignment operator (for example, x -= 3 corresponds to x = x - 3)


```

<a id="s-4-8"></a>

### § 4.8 Mathematical functions

```text
§   4.8. Mathematical functions
    The following mathematical functions (Abs, Clip3, Clip1, Min, Max, Round2 and Round2Signed) are defined as
    follows:


                                                                         x;      x≥ 0                                                 (1)
                                                       Abs(x) = {
                                                    (1)Abs(x)={x;x≥0−x;x<0
                                                                   − x; x < 0
                                                                   x;        x≥ 0                                                    (1)
                                                    Abs(x) = {
                                                                   − x;BitDepth
                                                             Clip3(0, 22BitD
                                                 Clip1(x)= =Clip3(0,
                                                                             xepth
                                                                               <− 01, x)                                              (2)
                                              Clip1(x)                             − 1, x)                                           (2)
                                              (2)Clip1(x)=Clip3(0,2BitDepth−1,x)

    AV2 Specification                                                                                                        Page 42 of 1169
                                                      ⎧x; z < x
                               (3)Clip3(x,y,z)={x;z<xy;z>yz;otherwise
                                     Clip3(x, y, z) = ⎨y; z > y                                        (3)
                                                      ⎧x; z < x
                                                      ⎩
                                                      ⎨z; ot herwise
                                   Clip3(x, y, z) = ⎩y; z > y                                         (3)
                                                        z; xot≤ herwise
                                                       x;          y                                   (4)
                                         Min(x, y) = {
                                       (4)Min(x,y)={x;x≤yy;x>y
                                                       y; x > y
                                                        x; x ≤ y                                      (4)
                                       Min(x, y) = {
                                                        y; x > y
                                                        x; x ≥ y                                       (5)
                                         Max(x, y) = {
                                       (5)Max(x,y)={x;x≥yy;x<y
                                                        y; x < y
                                                        x; x ≥ y                                      (5)
                                       Max(x, y) = {
                                                        y;x + x2n −<1 y
                                       Round2(x, n ) = ⌊              ⌋                                (6)
                                     (6)Round2(x,n)=⌊x+2n−12n⌋
                                                             2      n− 1
                                                           x+ 2                                       (6)
                                     Round2(x, n ) = ⌊                   ⌋
                                                                 n
                                                               2
                                                     Round2(x, n );           x≥ 0                     (7)
                             Round2Signed(x, n ) = {
                   (7)Round2Signed(x,n)={Round2(x,n);x≥0−Round2(−x,n);x<0
                                                     − Round2(− x, n ); x < 0
                                                     Round2(x,        n );       x≥ 0                 (7)
                       Round2Signed(x, n ) = {
                                                     − Round2(−
The definition of Round2 uses standard mathematical power   and divisionx, noperations,
                                                                             ); x < 0not integer
operations. An equivalent definition using integer operations is:

 Round2( x, n ) {
   if ( n == 0 )
     return x
   return (x + (1 << (n - 1)) ) >> n
 }


The FloorLog2(x) function is defined to be the floor of the base 2 logarithm of the input x.

The input x will always be an integer, and will always be greater than or equal to 1.




                                                       ⎪
This function extracts the location of the most significant bit (MSB) in x.

An equivalent definition (using the pseudo-code notation introduced in the following section) is:

 FloorLog2( x ) {




 }
   s = 0
   while ( x != 0 ) {


   }
     x = x >> 1
     s++

   return s - 1




The GetMsb(x) function is the same as FloorLog2, except that an input of 0 is also allowed.

The function is defined as follows:

 GetMsb( x ) {
   if ( x==0 ) {
     return 0




AV2 Specification                                                                              Page 43 of 1169
         }
         return FloorLog2( x )
     }


    The CeilLog2(x) function is defined to be the ceiling of the base 2 logarithm of the input x (when x is 0, it is
    defined to return 0).

    The input x will always be an integer, and will always be greater than or equal to 0.

    This function extracts the number of bits needed to code a value in the range 0 to x-1.

    An equivalent definition (using the pseudo-code notation introduced in the following section) is:

     CeilLog2( x ) {
       if ( x < 2 )
         return 0
       i = 1
       p = 2
       while ( p < x ) {
         i++
         p = p << 1
       }
       return i
     }


```

<a id="s-4-9"></a>

### § 4.9 Method of describing bitstream syntax

```text
§   4.9. Method of describing bitstream syntax
    The description style of the syntax is similar to the C programming language. Syntax elements in the
    bitstream are represented in bold type. Each syntax element is described by its name (using only lower
    case letters with underscore characters) and a descriptor for its method of coded representation. The
    decoding process behaves according to the value of the syntax element and to the values of previously
    decoded syntax elements. When a value of a syntax element is used in the syntax tables or the text, it
    appears in regular (i.e., not bold) type. If the value of a syntax element is being computed (e.g., being
    written with a default value instead of being coded in the bitstream), it also appears in regular type (e.g.,
    tile_size_minus_1).


    In some cases the syntax tables may use the values of other variables derived from syntax element
    values. Such variables appear in the syntax tables, or text, named by a mixture of lower case and upper
    case letters and without any underscore characters. Variables starting with an upper case letter are
    derived for the decoding of the current syntax structure and all dependent syntax structures. These
    variables may be used in the decoding process for later syntax structures. Variables starting with a lower
    case letter are only used within the process from which they are derived. (Single-character variables are
    allowed.)

    Constant values appear in all upper case letters with underscore characters (e.g., MI_SIZE).

    Constant lookup tables appear as words (with the first letter of each word in upper case, and remaining
    letters in lower case) separated with underscore characters (e.g., Block_Width[…]).

    Hexadecimal notation, indicated by prefixing the hexadecimal number by 0x, may be used when the
    number of bits is an integer multiple of 4. For example, 0x1a represents a bit string 0001 1010.

    Binary notation is indicated by prefixing the binary number by 0b. For example, 0b00011010 represents a bit
    string 0001 1010. Binary numbers may include underscore characters to enhance readability. If present,



    AV2 Specification                                                                                  Page 44 of 1169
the underscore characters appear every 4 binary digits starting from the LSB. For example, 0b11010 may
also be written as 0b1_1010.

A value equal to 0 represents a FALSE condition in a test statement. The TRUE condition is represented by
any value not equal to 0.

The following table lists examples of the syntax specification format. When syntax_element appears (in bold
font), it specifies that this syntax element is parsed from the bitstream.

 syntax_structure_name( parameter1, parameter2, ... ) {                                         Descriptor

 /* A statement can be a syntax element with an associated descriptor or can be an expression
 used to specify its existence, type, and value, as in the following examples. */

 syntax_element                                                                                    f(1)

 /* A group of statements enclosed in brackets is a compound statement and is treated
 functionally as a single statement. */

 {

     statement

     ...

 }

 /* A "while" structure specifies that the statement is to be evaluated repeatedly while the
 condition remains true. */

 while ( condition )

     statement

 /* A "do .. while" structure executes the statement once and then tests the condition. It
 repeatedly evaluates the statement while the condition remains true. */

 do

     statement

 while ( condition )

 /* An "if .. else" structure tests the condition first. If it is true, the primary statement
 is evaluated. Otherwise, the alternative statement is evaluated. If the alternative
 statement is unnecessary to be evaluated, the "else" and corresponding alternative statement
 can be omitted. */

 if ( condition )

     primary statement

 else

     alternative statement

 /* A "for" structure evaluates the initial statement at the beginning, then tests the
 condition. If it is true, the primary and subsequent statements are evaluated until the
 condition becomes false. */

 for ( initial statement; condition; subsequent statement )

     primary statement

 /* The return statement in a syntax structure specifies that the parsing of the syntax
 structure will be terminated without processing any additional information after this stage.
 When a value immediately follows a return statement, this value shall also be returned as
 the output of this syntax structure. */

 return x

 }




AV2 Specification                                                                               Page 45 of 1169
```

<a id="s-4-10"></a>

### § 4.10 Functions

```text
§   4.10. Functions
    Bitstream functions used for syntax description are specified in this section.

    Other functions are included in the syntax tables. The convention is that a section is called _syntax_ if it
    causes syntax elements to be read from the bitstream, either directly or indirectly through subprocesses.
    The remaining sections are called _functions_.

    The specification of these functions makes use of a bitstream position indicator. This bitstream position
    indicator locates the position of the bit that is going to be read next.

    get_position( ): Return the value of the bitstream position indicator.

    init_symbol( sz ): Initialize the arithmetic decode process for the symbol decoder with a size of sz bytes
    as specified in § 8.2.2 Initialization process for symbol decoder.

    exit_symbol( ): Exit the arithmetic decode process as described in § 8.2.4 Exit process for symbol
    decoder (this includes reading trailing bits).

    When referring to a function, brackets are included only when introducing a parameter which is needed
    for the explanation.

```

<a id="s-4-11"></a>

### § 4.11 Descriptors

```text
§   4.11. Descriptors
```

<a id="s-4-11-1"></a>

#### § 4.11.1 General

```text
§   4.11.1. General

    The following descriptors specify the parsing of syntax elements. Lower case descriptors specify syntax
    elements that are represented by an integer number of bits in the bitstream; upper case descriptors
    specify syntax elements that are represented by arithmetic coding.

```

<a id="s-4-11-2"></a>

#### § 4.11.2 f(n)

```text
§   4.11.2. f(n)

    Unsigned n-bit number appearing directly in the bitstream. The bits are read from highest to lowest. The
    parsing process specified in § 8.1 Parsing process for f(n) is invoked, and the syntax element is set equal
    to the return value.

```

<a id="s-4-11-3"></a>

#### § 4.11.3 uvlc()

```text
§   4.11.3. uvlc()

    Variable-length unsigned number appearing directly in the bitstream. The parsing process for this
    descriptor is specified below:

     uvlc() {                                                                                      Descriptor

       leadingZeros = 0

       while ( 1 ) {

           done                                                                                       f(1)

           if ( done )

            break

           leadingZeros++

       }

       if ( leadingZeros >= 32 ) {

           return ( 1 << 32 ) - 1



    AV2 Specification                                                                              Page 46 of 1169
         }

         value                                                                                      f(leadingZeros)

         return value + ( 1 << leadingZeros ) - 1

     }


    It is a requirement of bitstream conformance that leadingZeros is less than 32 when this function returns.


         NOTE:      This means that the largest value that can be returned by a uvlc() descriptor is ( 1 << 32 ) -
         2.


```

<a id="s-4-11-4"></a>

#### § 4.11.4 svlc()

```text
§   4.11.4. svlc()

    Variable-length signed number appearing directly in the bitstream. The parsing process for this
    descriptor is specified below:

     svlc() {                                                                                          Descriptor

         value                                                                                           uvlc()

         half = (value + 1) >> 1

         if (value & 1) {

             return half

         } else {

             return -half

         }

     }


```

<a id="s-4-11-5"></a>

#### § 4.11.5 le(n)

```text
§   4.11.5. le(n)

    Unsigned little-endian n-byte number appearing directly in the bitstream. The parsing process for this
    descriptor is specified below:

     le(n) {                                                                                           Descriptor

         t = 0

         for ( i = 0; i < n; i++) {

             byte                                                                                         f(8)

             t += ( byte << ( i * 8 ) )

         }

         return t

     }


```

<a id="s-4-11-6"></a>

#### § 4.11.6 leb128()

```text
§   4.11.6. leb128()

    Unsigned integer represented by a variable number of little-endian bytes.


         NOTE:      This syntax element will only be present when the bitstream position is byte aligned.




    AV2 Specification                                                                                  Page 47 of 1169
    In this encoding, the most significant bit of each byte is equal to 1 to signal that more bytes should be
    read, or equal to 0 to signal the end of the encoding.

    A variable Leb128Bytes is set equal to the number of bytes read during this process.

    The parsing process for this descriptor is specified below:

     leb128() {                                                                                      Descriptor

         value = 0

         Leb128Bytes = 0

         for ( i = 0; i < 8; i++ ) {

             leb128_byte                                                                                f(8)

             value |= ( (leb128_byte & 0x7f) << (i*7) )

             Leb128Bytes += 1

             if ( !(leb128_byte & 0x80) ) {

                 break

             }

         }

         return value

     }


    It is a requirement of bitstream conformance that the value returned from the leb128 parsing process is
    less than or equal to (1 << 32) - 1.

    leb128_byte contains 8 bits read from the bitstream. The bottom 7 bits are used to compute the variable
    value. The most significant bit is used to indicate that there are more bytes to be read.

    It is a requirement of bitstream conformance that the most significant bit of leb128_byte is equal to 0 if i is
    equal to 7. (This ensures that this syntax descriptor never uses more than 8 bytes.)


         NOTE: There are multiple ways of encoding the same value, depending on how many leading zero
         bits are encoded. There is no requirement that this syntax descriptor uses the most compressed
         representation. This can be useful for encoder implementations by allowing a fixed amount of space
         to be filled in later when the value becomes known.


         NOTE: Only 5 bytes (providing 35 bits) are needed for this syntax descriptor because the bitstream
         conformance requirement limits the return value to 32 bits (7 bits in each of the first 4 bytes, and 4
         bits in the 5th byte).

```

<a id="s-4-11-7"></a>

#### § 4.11.7 su(n)

```text
§   4.11.7. su(n)

    Signed integer converted from an n-bit unsigned integer in the bitstream. (The unsigned integer
    corresponds to the bottom n bits of the signed integer.) The parsing process for this descriptor is
    specified below:

     su(n) {                                                                                         Descriptor

         value                                                                                          f(n)




    AV2 Specification                                                                                 Page 48 of 1169
         signMask = 1 << (n - 1)

         if ( value & signMask )

             value = value - 2 * signMask

         return value

     }


```

<a id="s-4-11-8"></a>

#### § 4.11.8 ns(n)

```text
§   4.11.8. ns(n)

    Unsigned encoded integer with maximum number of values n (i.e., output in range 0..n-1).

    This descriptor is similar to f(CeilLog2(n)), but reduces wastage incurred when encoding non-power of
    two value ranges by encoding 1 fewer bit for the lower part of the value range. For example, when n is
    equal to 5, the encodings are as follows (full binary encodings are also presented for comparison):

                                            Table 4.1: Example encodings for ns(5)

               Value                        Full binary encoding                     ns(n) encoding

                  0                                 000                                   00

                  1                                 001                                   01

                  2                                 010                                   10

                  3                                 011                                   110

                  4                                 100                                   111


    The parsing process for this descriptor is specified as:

     ns( n ) {                                                                                    Descriptor

         w = FloorLog2(n) + 1

         m = (1 << w) - n

         v                                                                                            f(w - 1)

         if ( v < m )

             return v

         extra_bit                                                                                      f(1)

         return (v << 1) - m + extra_bit

     }


    The abbreviation ns stands for _non-symmetric_. This encoding is non-symmetric because the values are
    not all coded with the same number of bits.

```

<a id="s-4-11-9"></a>

#### § 4.11.9 tu(mx)

```text
§   4.11.9. tu(mx)

    Integer in the range 0 to mx using truncated unary encoding (a series of zero or more 1s followed by a
    single 0, except that the final 0 is omitted if the maximum is reached).

    The parsing process for this descriptor is specified below:

     tu( mx ) {                                                                                   Descriptor

         for ( idx = 0; idx < mx; idx++ ) {




    AV2 Specification                                                                             Page 49 of 1169
             tu_bit                                                                                    f(1)

             if ( tu_bit == 0 ) {

                 return idx

             }

         }

         return mx

     }


```

<a id="s-4-11-10"></a>

#### § 4.11.10 rg(n)

```text
§   4.11.10. rg(n)

    Integer with Rice-Golomb coding with parameter n (a fixed length coding of the n least significant bits
    preceded by a unary encoding of the most significant bits).

    The parsing process for this descriptor is specified below:

     rg( n ) {                                                                                      Descriptor

         for ( q = 0; q < 32; q++ ) {

             rg_bit                                                                                    f(1)

             if ( rg_bit == 0 ) {

                 remainder                                                                             f(n)

                 return (q << n) + remainder

             }

         }

         return -1

     }


    It is a requirement of bitstream conformance that this descriptor never returns a value less than 0.

```

<a id="s-4-11-11"></a>

#### § 4.11.11 L(n)

```text
§   4.11.11. L(n)

    Unsigned arithmetic encoded n-bit number encoded as n flags (a _literal_). The flags are read from
    highest to lowest. The syntax element is set equal to the return value of read_literal( n ) (see § 8.2.5
    Parsing process for read_literal for a specification of this process).

```

<a id="s-4-11-12"></a>

#### § 4.11.12 S()

```text
§   4.11.12. S()

    An arithmetic encoded symbol coded from a small alphabet of at most 8 entries.

    The symbol is decoded based on a context-sensitive CDF (see § 8.3 Parsing process for CDF encoded
    syntax elements for the specification of this process).

```

<a id="s-4-11-13"></a>

#### § 4.11.13 NS(n)

```text
§   4.11.13. NS(n)

    Unsigned arithmetic encoded integer with maximum number of values n (i.e., output in range 0..n-1).

    This descriptor is the same as ns(n), except the underlying bits are coded arithmetically.

    The parsing process for this descriptor is specified as:




    AV2 Specification                                                                               Page 50 of 1169
 NS( n ) {                                         Descriptor

     w = FloorLog2(n) + 1

     m = (1 << w) - n

     v                                              L(w - 1)

     if ( v < m )

         return v

     extra_bit                                        L(1)

     return (v << 1) - m + extra_bit

 }


                                       ↑ Back to Table of Contents




AV2 Specification                                   Page 51 of 1169
```
