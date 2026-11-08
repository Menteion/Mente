---
id: isqrt
milestone: M1
core: true
set: public
kind: solve
---

# Integer square root

## Task

Return the integer square root of `n`: the largest integer `r` such that `r * r <= n`. Every `u64` value is a valid input, the largest included.

## Signature

    isqrt(n: u64) -> u64

## Examples

    isqrt(0) == 0
    isqrt(15) == 3
    isqrt(16) == 4

## Hidden tests

    isqrt(1) == 1
    isqrt(2) == 1
    isqrt(3) == 1
    isqrt(4) == 2
    isqrt(24) == 4
    isqrt(25) == 5
    isqrt(26) == 5
    isqrt(99) == 9
    isqrt(100) == 10
    isqrt(4294967295) == 65535
    isqrt(4294967296) == 65536
    isqrt(18446744073709551615) == 4294967295
    isqrt(18446744065119617025) == 4294967295
    isqrt(18446744065119617024) == 4294967294
    isqrt(18446744073709551614) == 4294967295
    isqrt(1000000000000000000) == 1000000000
    isqrt(999999999999999999) == 999999999
    isqrt(12345678987654321) == 111111111
