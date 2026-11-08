---
id: encode-sample
milestone: M3
core: false
set: public
kind: solve
---

# Encode a sensor sample

## Task

Encode `s` as the 11 bytes its layout describes: the fields in declaration order, packed with no padding, each multi-byte field big-endian. Declare `Sample` yourself, with exactly this layout and these field names: the harness builds its values field by field.

## Types

    layout record Sample (11 bytes, big-endian, packed) {
        sensor: u16    // bytes 0 and 1
        value: i32     // bytes 2 to 5, two's complement
        flags: u8      // byte 6
        time: u32      // bytes 7 to 10
    }

## Signature

    encode_sample(s: Sample) -> [11]u8

## Examples

    encode_sample({sensor: 1, value: 2, flags: 3, time: 4}) == [0, 1, 0, 0, 0, 2, 3, 0, 0, 0, 4]
    encode_sample({sensor: 65535, value: -1, flags: 255, time: 4294967295}) == [255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255]
    encode_sample({sensor: 258, value: -2147483648, flags: 0, time: 16909060}) == [1, 2, 128, 0, 0, 0, 0, 1, 2, 3, 4]

## Hidden tests

    encode_sample({sensor: 0, value: 0, flags: 0, time: 0}) == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    encode_sample({sensor: 4660, value: 305419896, flags: 171, time: 3735928559}) == [18, 52, 18, 52, 86, 120, 171, 222, 173, 190, 239]
    encode_sample({sensor: 256, value: 2147483647, flags: 128, time: 1}) == [1, 0, 127, 255, 255, 255, 128, 0, 0, 0, 1]
    encode_sample({sensor: 1, value: -256, flags: 1, time: 16777216}) == [0, 1, 255, 255, 255, 0, 1, 1, 0, 0, 0]
    encode_sample({sensor: 32768, value: -65536, flags: 15, time: 65535}) == [128, 0, 255, 255, 0, 0, 15, 0, 0, 255, 255]
