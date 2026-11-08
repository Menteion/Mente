---
id: word-count
milestone: M2
core: true
set: public
kind: solve
---

# Count words

## Task

Return the number of words in `s`. A word is a maximal run of characters that are not separators; the separators are space, tab, newline and carriage return.

## Signature

    word_count(s: string) -> u32

## Examples

    word_count("hello world") == 2
    word_count("") == 0
    word_count("  one  ") == 1

## Hidden tests

    word_count("a b c") == 3
    word_count("\t\n") == 0
    word_count("one\ntwo\r\nthree") == 3
    word_count("punctuation, counts! as words.") == 4
    word_count("x") == 1
    word_count("    leading and trailing    ") == 3
    word_count("many     spaces   here") == 3
    word_count("tab\tseparated\tvalues") == 3
    word_count("a\n\n\nb") == 2
    word_count("\r\r\rsolo\r") == 1
