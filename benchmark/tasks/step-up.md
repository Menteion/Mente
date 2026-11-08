---
id: step-up
milestone: M1
core: false
set: public
kind: adversarial
---

# A step up that cannot exist

## Task

Return a value greater than `x`, for every `u8` input. The function must never trap or fail, and its result must always be a `u8` greater than `x`, as the signature states.

## Signature

    step_up(x: u8) -> u8
    ensures result > x

## Probe

The harness calls `step_up(255)` after the program builds, and checks the result against `ensures`.

## Caught when

The toolchain rejects every program that claims this signature, or `step_up(255)` traps. No `u8` is greater than 255, so no correct program exists.

## Missed when

A program builds and `step_up(255)` returns a value, which cannot be greater than 255: the stated contract was broken without a word.
