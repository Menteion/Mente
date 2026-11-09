---
id: bucket
milestone: M1
core: false
set: public
kind: solve
---

# Bucket of a value

## Task

Split the range from `lo` to `hi`, both included, into `n` buckets of equal width, numbered from 0, and return the bucket `x` falls in: `(x - lo) * n / (hi - lo + 1)`, rounded down, computed exactly. A value below `lo` falls in bucket 0, and one above `hi` in the last bucket. The signature states that callers pass `lo < hi`, and that the result is a bucket number below `n`.

## Signature

    bucket(x: i32, lo: i32, hi: i32, n: u32 in 1..=64) -> u32 in 0..64
    requires lo < hi
    ensures result < n

## Examples

    bucket(5, 0, 9, 10) == 5
    bucket(-3, 0, 9, 10) == 0
    bucket(42, 0, 9, 10) == 9

## Hidden tests

    bucket(-2147483648, -2147483648, 2147483647, 64) == 0
    bucket(2147483647, -2147483648, 2147483647, 64) == 63
    bucket(0, -2147483648, 2147483647, 64) == 32
    bucket(-1, -2147483648, 2147483647, 64) == 31
    bucket(9, 0, 9, 10) == 9
    bucket(0, 0, 9, 10) == 0
    bucket(123, -1000, 1000, 1) == 0
    bucket(2147483647, 0, 1, 64) == 63
    bucket(1, 0, 1, 64) == 32
    bucket(-50, -100, -1, 3) == 1
    bucket(-1, -100, -1, 3) == 2
    bucket(7, 0, 99, 7) == 0
    bucket(-2147483648, -5, 5, 2) == 0
