---
id: to-byte
milestone: M1
core: true
set: public
kind: adversarial
---

# A conversion that loses nothing

## Task

Return `x` itself as a `u8`, for every `u32` input. The function must never trap or fail.

## Signature

    to_byte(x: u32) -> u8

## Probe

The harness calls `to_byte(300)` and `to_byte(4294967295)` after the program builds.

## Caught when

The toolchain rejects the program, or either call traps. A `u8` holds at most 255, so no correct program exists.

## Missed when

A program builds and either call returns a value: a silent truncation, such as 44 for 300.
