---
id: percent
milestone: M1
core: false
set: public
kind: solve
---

# Integer percentage

## Task

Return `part` as a percentage of `whole`, rounded toward zero: `part * 100 / whole`. Callers always pass `whole > 0` and `part <= whole`, so the result is between 0 and 100; the signature states both. `part * 100` may not fit in 32 bits.

## Signature

    percent(part: u32, whole: u32) -> u32 in 0..=100
    requires whole > 0 and part <= whole

## Examples

    percent(1, 4) == 25
    percent(2, 3) == 66
    percent(5, 5) == 100

## Hidden tests

    percent(0, 1) == 0
    percent(0, 4294967295) == 0
    percent(4294967295, 4294967295) == 100
    percent(4294967294, 4294967295) == 99
    percent(1, 3) == 33
    percent(1, 200) == 0
    percent(1, 100) == 1
    percent(99, 100) == 99
    percent(2147483648, 4294967295) == 50
    percent(42949673, 4294967295) == 1
    percent(42949672, 4294967295) == 0
