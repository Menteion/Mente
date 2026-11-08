---
id: rpn-eval
milestone: M2
core: true
set: public
kind: solve
---

# Evaluate reverse Polish notation

## Task

Evaluate the expression in reverse Polish notation that `tokens` holds. A token is either an integer, written in decimal with an optional leading `-` (`-3` is the number minus three), or one of the operators `+`, `-`, `*` and `/`. An operator takes the two topmost values, the lower one as its left operand, and pushes its result; `/` rounds toward zero. Every value must fit in an `i64`.

Return the error for the first token that fails: `BadToken` for a token that is neither an integer nor an operator, `StackUnderflow` for an operator with fewer than two values, `DivisionByZero`, and `Overflow` for a number or a result outside `i64`. After the last token exactly one value must remain: return `StackUnderflow` when none remains and `Leftover` when more than one remains.

## Types

    enum EvalError { BadToken, StackUnderflow, DivisionByZero, Overflow, Leftover }

## Signature

    rpn_eval(tokens: slice<string>) -> result<i64, EvalError>

## Examples

    rpn_eval(["2", "3", "+", "4", "*"]) == ok(20)
    rpn_eval(["1", "+"]) == err(StackUnderflow)
    rpn_eval(["7", "-2", "/"]) == ok(-3)

## Hidden tests

    rpn_eval([]) == err(StackUnderflow)
    rpn_eval(["5"]) == ok(5)
    rpn_eval(["1", "2"]) == err(Leftover)
    rpn_eval(["1", "0", "/"]) == err(DivisionByZero)
    rpn_eval(["9223372036854775807", "1", "+"]) == err(Overflow)
    rpn_eval(["-9223372036854775808", "-1", "/"]) == err(Overflow)
    rpn_eval(["-9223372036854775808"]) == ok(-9223372036854775808)
    rpn_eval(["9223372036854775808"]) == err(Overflow)
    rpn_eval(["3", "x", "+"]) == err(BadToken)
    rpn_eval(["3", "4", "^"]) == err(BadToken)
    rpn_eval(["+3"]) == err(BadToken)
    rpn_eval(["-"]) == err(StackUnderflow)
    rpn_eval(["10", "3", "-"]) == ok(7)
    rpn_eval(["10", "3", "/"]) == ok(3)
    rpn_eval(["-7", "2", "/"]) == ok(-3)
    rpn_eval(["3", "4", "+", "2", "1", "-", "*"]) == ok(7)
    rpn_eval(["4294967296", "4294967296", "*"]) == err(Overflow)
    rpn_eval(["3037000499", "3037000499", "*"]) == ok(9223372030926249001)
    rpn_eval(["1", "0", "/", "x"]) == err(DivisionByZero)
    rpn_eval(["x", "1", "0", "/"]) == err(BadToken)
    rpn_eval(["0", "-0", "-"]) == ok(0)
