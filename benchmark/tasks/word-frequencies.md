---
id: word-frequencies
milestone: M6
core: true
set: public
kind: solve
---

# Word frequencies

## Task

Count the words in `text`. A word is a maximal run of the ASCII letters `a` to `z` and `A` to `Z`, and case does not matter: `The` and `the` are the same word, written in lower case in the result. Return one entry per distinct word, sorted by count from highest to lowest, and words with equal counts in alphabetical order.

## Types

    record WordCount { word: string, count: u32 }

## Signature

    word_frequencies(text: string) -> list<WordCount>

## Examples

    word_frequencies("the cat and the hat") == [{word: "the", count: 2}, {word: "and", count: 1}, {word: "cat", count: 1}, {word: "hat", count: 1}]
    word_frequencies("") == []
    word_frequencies("A a A b") == [{word: "a", count: 3}, {word: "b", count: 1}]

## Hidden tests

    word_frequencies("Hello, world! Hello?") == [{word: "hello", count: 2}, {word: "world", count: 1}]
    word_frequencies("it's") == [{word: "it", count: 1}, {word: "s", count: 1}]
    word_frequencies("x1y2z") == [{word: "x", count: 1}, {word: "y", count: 1}, {word: "z", count: 1}]
    word_frequencies("zeta alpha beta alpha zeta zeta") == [{word: "zeta", count: 3}, {word: "alpha", count: 2}, {word: "beta", count: 1}]
    word_frequencies("   ") == []
    word_frequencies("Tab\tand\nnewline") == [{word: "and", count: 1}, {word: "newline", count: 1}, {word: "tab", count: 1}]
    word_frequencies("b a c b a c b") == [{word: "b", count: 3}, {word: "a", count: 2}, {word: "c", count: 2}]
    word_frequencies("Aa aA AA aa") == [{word: "aa", count: 4}]
