---
id: gcd
milestone: M1
core: true
set: public
kind: solve
---

# Greatest common divisor

## Task

Return the greatest common divisor of `a` and `b`. The greatest common divisor of `n` and 0 is `n`, and of 0 and 0 is 0.

## Signature

    gcd(a: u64, b: u64) -> u64

## Examples

    gcd(12, 18) == 6
    gcd(0, 5) == 5
    gcd(17, 5) == 1

## Hidden tests

    gcd(0, 0) == 0
    gcd(5, 0) == 5
    gcd(1, 1) == 1
    gcd(18446744073709551615, 1) == 1
    gcd(18446744073709551615, 18446744073709551615) == 18446744073709551615
    gcd(18446744073709551614, 9223372036854775807) == 9223372036854775807
    gcd(1071, 462) == 21
    gcd(462, 1071) == 21
    gcd(1000000007, 998244353) == 1
    gcd(4294967296, 65536) == 65536
    gcd(720720, 360360) == 360360
    gcd(9223372036854775808, 6148914691236517205) == 1
    gcd(12345678901234567890, 9876543210) == 90
