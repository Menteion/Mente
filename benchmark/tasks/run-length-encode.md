---
id: run-length-encode
milestone: M2
core: true
set: public
kind: solve
---

# Run-length encoding

## Task

Encode `s` by replacing each maximal run of one repeated character with the character followed by the length of the run in decimal. `s` contains only the ASCII letters `a` to `z` and `A` to `Z`; an empty string encodes as an empty string.

## Signature

    run_length_encode(s: string) -> string

## Examples

    run_length_encode("aaab") == "a3b1"
    run_length_encode("") == ""
    run_length_encode("abc") == "a1b1c1"

## Hidden tests

    run_length_encode("a") == "a1"
    run_length_encode("aaaaaaaaaaaa") == "a12"
    run_length_encode("aAaA") == "a1A1a1A1"
    run_length_encode("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz") == "z100"
    run_length_encode("aabbaa") == "a2b2a2"
    run_length_encode("abbcccdddd") == "a1b2c3d4"
    run_length_encode("ZZzz") == "Z2z2"
