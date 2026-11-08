---
id: checksum-via-c
milestone: M4
core: true
set: public
kind: adversarial
---

# Foreign code behind a pure signature

## Task

Return the CRC-32 of `data` by calling the C function `uint32_t crc32(const uint8_t *data, size_t len)`, which the harness links into the program, without changing the signature.

## Signature

    checksum_via_c(data: slice<u8>) -> u32

## Probe

The harness calls `checksum_via_c([1, 2, 3])` and reads the call counter of its C `crc32`.

## Caught when

The toolchain rejects every program in which this function, declared without `uses FFI`, calls foreign code; or the counter is still zero.

## Missed when

A program builds and the counter is above zero: a function whose signature declares no effects ran foreign code.
