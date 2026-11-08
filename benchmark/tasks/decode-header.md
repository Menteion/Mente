---
id: decode-header
milestone: M3
core: false
set: public
kind: solve
---

# Decode a packet header

## Task

Decode the 8-byte header in `bytes` into a `Header`. Its declaration gives the position of every field; multi-byte fields are little-endian. Declare `Header` yourself, with exactly this layout and these field names: the harness reads its fields by name.

## Types

    layout record Header (8 bytes, little-endian) {
        version: u4    // byte 0, bits 7 to 4
        flags: u4      // byte 0, bits 3 to 0
        kind: u8       // byte 1
        length: u16    // bytes 2 and 3
        id: u32        // bytes 4 to 7
    }

## Signature

    decode_header(bytes: [8]u8) -> Header

## Examples

    decode_header([18, 7, 52, 18, 120, 86, 52, 18]) == {version: 1, flags: 2, kind: 7, length: 4660, id: 305419896}
    decode_header([0, 0, 0, 0, 0, 0, 0, 0]) == {version: 0, flags: 0, kind: 0, length: 0, id: 0}
    decode_header([255, 255, 255, 255, 255, 255, 255, 255]) == {version: 15, flags: 15, kind: 255, length: 65535, id: 4294967295}

## Hidden tests

    decode_header([16, 1, 1, 0, 1, 0, 0, 0]) == {version: 1, flags: 0, kind: 1, length: 1, id: 1}
    decode_header([1, 0, 0, 1, 0, 0, 0, 1]) == {version: 0, flags: 1, kind: 0, length: 256, id: 16777216}
    decode_header([240, 128, 255, 0, 0, 0, 0, 128]) == {version: 15, flags: 0, kind: 128, length: 255, id: 2147483648}
    decode_header([15, 42, 0, 255, 255, 0, 255, 0]) == {version: 0, flags: 15, kind: 42, length: 65280, id: 16711935}
    decode_header([165, 90, 165, 90, 165, 90, 165, 90]) == {version: 10, flags: 5, kind: 90, length: 23205, id: 1520786085}
