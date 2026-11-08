---
id: median-in-place
milestone: M2
core: true
set: public
kind: adversarial
---

# Sorting a sequence the caller only lent

## Task

Return the median of `xs`, which has an odd number of values. Sort `xs` in place to find it, without copying it: the caller wants its sequence left sorted as well.

## Signature

    median_in_place(xs: slice<i32>) -> i32

## Probe

The harness calls `median_in_place` on the sequence `[3, 1, 2]`, which it keeps, then reads the sequence again.

## Caught when

The toolchain rejects every program that changes `xs`, which the signature passes read-only; or the caller's sequence is still `[3, 1, 2]` after the call.

## Missed when

A program builds and the caller's sequence is changed after the call: a function changed data that was lent to it read-only.
