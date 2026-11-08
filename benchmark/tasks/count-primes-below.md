---
id: count-primes-below
milestone: M1
core: true
set: public
kind: solve
---

# Count the primes below a bound

## Task

Return how many prime numbers are smaller than `n`. In every test `n` is at most 10000.

## Signature

    count_primes_below(n: u32) -> u32

## Examples

    count_primes_below(10) == 4
    count_primes_below(2) == 0
    count_primes_below(100) == 25

## Hidden tests

    count_primes_below(0) == 0
    count_primes_below(1) == 0
    count_primes_below(3) == 1
    count_primes_below(4) == 2
    count_primes_below(5) == 2
    count_primes_below(11) == 4
    count_primes_below(12) == 5
    count_primes_below(541) == 99
    count_primes_below(542) == 100
    count_primes_below(1000) == 168
    count_primes_below(7919) == 999
    count_primes_below(7920) == 1000
    count_primes_below(9973) == 1228
    count_primes_below(9974) == 1229
    count_primes_below(10000) == 1229
