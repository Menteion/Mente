---
id: q-mul
milestone: M5
core: false
set: public
kind: solve
---

# Fixed-point multiplication

## Task

`a` and `b` are signed fixed-point numbers with 16 integer bits and 16 fraction bits (Q16.16): the 32-bit raw value, read as a two's-complement integer, is the number times 65536, and the tests write a value as `q(raw)`. Return their product as a Q16.16 number: the exact product rounded to the nearest representable value, with ties rounded away from zero, and limited to the largest or the smallest Q16.16 value when it falls outside the range.

## Signature

    q_mul(a: fixed<16, 16>, b: fixed<16, 16>) -> fixed<16, 16>

## Examples

    q_mul(q(98304), q(131072)) == q(196608)
    q_mul(q(-65536), q(32768)) == q(-32768)
    q_mul(q(1), q(1)) == q(0)

## Hidden tests

    q_mul(q(32768), q(1)) == q(1)
    q_mul(q(-32768), q(1)) == q(-1)
    q_mul(q(2147483647), q(2147483647)) == q(2147483647)
    q_mul(q(-2147483648), q(2147483647)) == q(-2147483648)
    q_mul(q(-2147483648), q(-2147483648)) == q(2147483647)
    q_mul(q(2147483647), q(65536)) == q(2147483647)
    q_mul(q(-2147483648), q(65536)) == q(-2147483648)
    q_mul(q(-2147483648), q(-65536)) == q(2147483647)
    q_mul(q(16384), q(16384)) == q(4096)
    q_mul(q(3), q(21845)) == q(1)
    q_mul(q(65537), q(65535)) == q(65536)
    q_mul(q(0), q(-2147483648)) == q(0)
    q_mul(q(-98304), q(-98304)) == q(147456)
    q_mul(q(98303), q(-1)) == q(-1)
