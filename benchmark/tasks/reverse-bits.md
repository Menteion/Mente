---
id: reverse-bits
milestone: M1
core: true
set: public
kind: solve
---

# Reverse the bits of a byte

## Task

Return `x` with the order of its 8 bits reversed: bit 0 becomes bit 7, bit 1 becomes bit 6, and so on.

## Signature

    reverse_bits(x: u8) -> u8

## Examples

    reverse_bits(1) == 128
    reverse_bits(6) == 96
    reverse_bits(255) == 255

## Hidden tests

    reverse_bits(0) == 0
    reverse_bits(128) == 1
    reverse_bits(2) == 64
    reverse_bits(64) == 2
    reverse_bits(15) == 240
    reverse_bits(240) == 15
    reverse_bits(170) == 85
    reverse_bits(85) == 170
    reverse_bits(129) == 129
    reverse_bits(200) == 19
    reverse_bits(19) == 200
    reverse_bits(254) == 127
    reverse_bits(127) == 254
