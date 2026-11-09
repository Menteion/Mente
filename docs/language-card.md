---
title: Mente Language Card
version: 0.1.0
status: draft
date: 2026-11-09
---

# Mente Language Card (draft)

How to write Mente as of milestone M1, for an agent that has never seen it. This is the draft the syntax probe uses ([`syntax-probe`](design-review.md#syntax-probe)); the first version closes M1 ([`language-card`](design-review.md#language-card)). Every `mente` block below passes `mn check`, which a test holds it to. The full rules are in `spec/core-calculus.md` and `spec/grammar.md`.

## What M1 has

Integers, `bool`, fixed-size arrays `[N]T`, functions with read-only and `mut` parameters, `val` and `var`, `if`, `while` with `break` and `continue`, `guard`, `return`, `requires` and `ensures`, refined integer types such as `u8[0..10]`, and `todo`. Nothing else: no strings, floats, structs, enums, slices, `for` loops, methods, generics, closures or error types yet. Using one is an error that names the milestone that brings it.

## A module

```mente
module demo

/// The larger of two values.
pub fn larger(a: i32, b: i32) -> i32 {
  if a > b {
    a
  } else {
    b
  }
}

pub fn square(x: i64) -> i64 = x * x
```

- A file is a module: `module name`, then declarations. `pub` makes a function visible outside the module.
- A function's body is a block, or `= expression` for one expression.
- A block's value is its last expression. `return value` leaves early.
- A statement ends at the end of its line. There are no semicolons.
- A line goes on to the next only when it ends with an operator, `,`, `(`, `[`, `{` or `=`, or inside parentheses and brackets. `else`, `{`, `invariant` and `decreases` may start their own line.

## Bindings

```mente
module demo

pub fn total(a: [4]u32) -> u32 {
  val first = a[0]
  var sum: u32 = first
  sum += a[1] + a[2] + a[3]
  sum
}
```

- `val` never changes; `var` can be assigned. Write `val` and `var`, never `let` or `let mut`.
- A name is declared once in a function, its parameters and inner blocks included: no shadowing. Use a new name.
- Assignment `x = e`, element assignment `a[i] = e`, and `+= -= *= /= %= &= |= ^= <<= >>=` apply to a `var` or a `mut` parameter.

## Types

- Integer types `i8` `i16` `i32` `i64` `i128`, `u8` `u16` `u32` `u64` `u128`, and `usize` (64 bits, unsigned). `bool`.
- `[N]T` is an array of `N` values of type `T`; `N` is an integer literal. `a.len` is `N`. `[0; 8]` is eight zeros; `[1, 2, 3]` lists the elements.
- An array is a value: binding, passing and returning it copies it. Two names never share one.
- A refined type narrows an integer type: `u8[0..10]` holds 0 to 9, `i32[-5..=5]` holds -5 to 5. A bound is an integer literal, a `val` or read-only parameter, or `name.len`.
- Literals take the type around them. `255_u8` states it.

## Integer arithmetic

- `+ - * / %` compute the exact result, whatever the types; `/` truncates toward zero, `%` takes the sign of the left operand.
- A value is checked against its type where it lands: a binding, an assignment, an argument, a returned value, an array element. A value outside the range traps at run time, or is an error at compile time if the type is refined and the compiler cannot prove it fits.
- There are no casts and no `as`: any integer expression may flow into any integer type, and is checked where it lands.
- `val x = a + b` takes the operands' type when they share one. When they differ, state the type: `val x: u64 = a + b`.
- A zero divisor traps, and so does an index outside its array.

## Operators

From loosest to tightest:

| Operators | Meaning |
| :--- | :--- |
| `or` | logical or |
| `and` | logical and |
| `not` | logical not |
| `==` `!=` `<` `<=` `>` `>=` | comparison; they do not chain |
| `\|` | bitwise or |
| `^` | bitwise xor |
| `&` | bitwise and |
| `<<` `>>` | shifts |
| `+` `-` | sum |
| `*` `/` `%` | product |
| `-` `~` | negation, bitwise not |
| `a[i]` `a.len` | index, length |

- Write `and`, `or`, `not`, never `&&`, `||` or `!`.
- `not a == b` means `not (a == b)`. `x & mask == 0` means `(x & mask) == 0`.
- `a < b < c` is an error: write `a < b and b < c`.

## Control flow

```mente
module demo

pub fn count_down(n: u32) -> u32 {
  var i = n
  var steps: u32 = 0
  while i > 0 decreases i {
    i -= 1
    if i == 3 {
      continue
    }
    steps += 1
  }
  steps
}

pub fn safe_div(a: i32, b: i32) -> i32 {
  guard b != 0 else {
    return 0
  }
  a / b
}

pub fn sign(x: i32) -> i32 = if x < 0 { -1 } else if x > 0 { 1 } else { 0 }
```

- `if c { ... } else { ... }` is an expression when it has `else`. An `if` inside an operator's operand needs parentheses: `1 + (if c { 2 } else { 3 })`.
- `while c { ... }` repeats while `c` holds. `break` leaves the loop, and `continue` starts its next round. Clauses come after the condition: `while lo < hi invariant lo <= hi decreases hi - lo {`.
- `guard c else { ... }` continues when `c` holds. Its block must leave: `return`, `break`, `continue` or `todo`.
- There is no `for`, `match`, ternary `? :`, `loop` or `do`.

## Functions and `mut`

```mente
module demo

pub fn swap(mut a: [4]i32, i: usize[0..4], j: usize[0..4]) {
  val t = a[i]
  a[i] = a[j]
  a[j] = t
}

pub fn demo() -> i32 {
  var v: [4]i32 = [1, 2, 3, 4]
  swap(mut v, 0, 3)
  v[0]
}
```

- Parameters are read-only by default. A `mut` parameter receives a place, marked at the call: `swap(mut v, 0, 3)`. It is copied in when the call starts and copied back when it returns.
- Never write `&x` or `&mut x`. There are no references, pointers or lifetimes.
- In one call, a `mut` argument's variable may not appear in any other argument: `f(mut a[i], a[j])` is an error.
- A function that returns nothing has no `->`.

## Contracts

```mente
module demo

pub fn clamp(x: i32, lo: i32, hi: i32) -> i32
  requires lo <= hi
  ensures lo <= result and result <= hi
{
  if x < lo {
    lo
  } else if x > hi {
    hi
  } else {
    x
  }
}
```

- `requires` holds at every call, and `ensures` at every return, where `result` is the returned value.
- The compiler proves contracts, refinements and loop invariants with simple interval and difference reasoning. When it cannot, it reports a value it could not rule out and suggests a `guard`. A plain integer type needs no proof: it is checked at run time.

## Unfinished work

`todo("reason")` stands for any value and traps when it runs. Use it instead of a placeholder that looks finished.

## Habits from other languages that Mente rejects

```text
let x = 1                // val x = 1
let mut y = 2            // var y = 2
x = 1;                   // x = 1 (no semicolons)
f(&mut v)                // f(mut v)
a && b, a || b, !a       // a and b, a or b, not a
c ? a : b                // if c { a } else { b }
x as u8                  // val y: u8 = x (checked where it lands)
for i in 0..n { }        // a while loop with a counter
```
