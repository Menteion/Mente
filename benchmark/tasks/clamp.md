---
id: clamp
milestone: M1
core: false
set: public
kind: solve
---

# Clamp to a range

## Task

Return `x` limited to the range from `lo` to `hi`: `lo` when `x` is below it, `hi` when `x` is above it, and `x` otherwise. Callers always pass `lo <= hi`; the signature states this, and states that the result lies within the range.

## Signature

    clamp(x: i32, lo: i32, hi: i32) -> i32
    requires lo <= hi
    ensures lo <= result and result <= hi

## Examples

    clamp(5, 0, 10) == 5
    clamp(-3, 0, 10) == 0
    clamp(42, 0, 10) == 10

## Hidden tests

    clamp(0, 0, 0) == 0
    clamp(-2147483648, -2147483648, 2147483647) == -2147483648
    clamp(2147483647, -5, 5) == 5
    clamp(-2147483648, -5, 5) == -5
    clamp(7, 7, 7) == 7
    clamp(10, 0, 10) == 10
    clamp(0, 0, 10) == 0
    clamp(-1, -10, -2) == -2
    clamp(-20, -10, -2) == -10
    clamp(3, -2147483648, -2147483648) == -2147483648
    clamp(2147483647, 2147483647, 2147483647) == 2147483647
