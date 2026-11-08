---
id: parse-u32
milestone: M2
core: true
set: public
kind: solve
---

# Parse an unsigned 32-bit number

## Task

Parse `s` as a decimal number. `s` must consist of the digits 0 to 9 only: no sign, no spaces and no other characters; leading zeros are allowed. Return `Empty` for an empty string, `InvalidDigit` when any character is not a digit, and `Overflow` when every character is a digit but the number is larger than the largest `u32`, 4294967295.

## Types

    enum ParseError { Empty, InvalidDigit, Overflow }

## Signature

    parse_u32(s: string) -> result<u32, ParseError>

## Examples

    parse_u32("42") == ok(42)
    parse_u32("") == err(Empty)
    parse_u32("12a") == err(InvalidDigit)

## Hidden tests

    parse_u32("0") == ok(0)
    parse_u32("007") == ok(7)
    parse_u32("4294967295") == ok(4294967295)
    parse_u32("4294967296") == err(Overflow)
    parse_u32("99999999999999999999") == err(Overflow)
    parse_u32("-1") == err(InvalidDigit)
    parse_u32("+1") == err(InvalidDigit)
    parse_u32(" 1") == err(InvalidDigit)
    parse_u32("1 ") == err(InvalidDigit)
    parse_u32("1.5") == err(InvalidDigit)
    parse_u32("1e3") == err(InvalidDigit)
    parse_u32("0x10") == err(InvalidDigit)
    parse_u32("00000000004294967295") == ok(4294967295)
    parse_u32("00000000004294967296") == err(Overflow)
    parse_u32("abc") == err(InvalidDigit)
    parse_u32("4294967295000") == err(Overflow)
