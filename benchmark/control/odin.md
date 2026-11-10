---
title: Odin for the Agent Benchmark
language: Odin
size: the language card's, which a test compares
date: 2026-11-10
---

# Odin for the agent benchmark

How to write the Odin a benchmark task needs, for an agent that has seen little of it. The agent benchmark gives this document to the agent that writes Odin, as the language card is given to the one that writes Mente, and of about the same size ([`control-language`](../../docs/design-review.md#control-language)). It covers the subset the M1 tasks use. Every `odin` block below is checked by the Odin compiler.

## What the tasks use

Integers, `bool`, fixed-size arrays `[N]T`, procedures, local variables and constants, `if`, `for` loops with `break` and `continue`, `switch`, pointers for parameters a procedure changes, and `assert` and `panic`. A task never needs strings, slices, dynamic arrays, maps, structs, unions or the standard library: the harness writes the program around your procedure and prints its result.

## A package

```odin
package demo

// The larger of two values.
larger :: proc(a: i32, b: i32) -> i32 {
	if a > b {
		return a
	}
	return b
}

square :: proc(x: i64) -> i64 {
	return x * x
}
```

- A file starts with `package name`, then declarations.
- `name :: proc(params) -> T { ... }` declares a procedure. A procedure that returns nothing has no `->`.
- A procedure returns its result with `return value`. The last expression is not a result: every path of a procedure with a result ends in `return`.
- Semicolons are optional at the end of a line, and usually left out. Braces are required around every block.
- Comments are `//` to the end of the line, or `/* ... */`.

## Variables and constants

```odin
package demo

total :: proc(a: [4]u32) -> u32 {
	first := a[0]
	sum: u32 = first
	sum += a[1] + a[2] + a[3]
	return sum
}

N :: 8
```

- `x := value` declares a variable whose type is the value's. `x: T = value` states the type, and `x: T` declares one set to zero.
- `NAME :: value` declares a constant, known when the program compiles.
- Every variable can be assigned: `x = e`, and `+= -= *= /= %= &= |= ~= <<= >>=`.
- A procedure's parameters cannot be assigned. To change a copy, declare a variable from it: `n := n`, which shadows the parameter.
- A name may be declared again in an inner block, hiding the outer one.

## Types

- Integer types `i8` `i16` `i32` `i64` `i128`, `u8` `u16` `u32` `u64` `u128`, `int` and `uint` (the platform's word, 64 bits here; `uint` is unsigned). `bool`, with `true` and `false`.
- `[N]T` is an array of `N` values of type `T`, where `N` is a constant. `len(a)` is `N`. `[4]u32{1, 2, 3, 4}` lists the elements, and `[8]i32{}` is eight zeros.
- An array is a value: assigning, passing and returning it copies it.
- An integer literal takes the type its place needs. Alone, `x := 5` makes an `int`.
- There are no implicit conversions between integer types: `a + b` needs `a` and `b` of one type. Convert with `T(x)`, e.g. `i64(a) + i64(b)`, or `cast(T)x`.

## Integer arithmetic

- `+ - * / %` work in the operands' type. At run time an overflow wraps around, two's complement, silently, for signed and unsigned types alike: for `x: u8 = 255`, `x + 1` is `0`. A constant expression that overflows is a compile-time error instead. Compute in a wider type when a result may not fit, then convert back.
- `/` truncates toward zero, and `%` takes the sign of the left operand. `%%` is the remainder that takes the sign of the right operand.
- A conversion `T(x)` keeps the low bits when `T` is narrower: for `x := 300`, `u8(x)` is `44`. Check the range first when a value may not fit.
- Dividing an integer by zero stops the program.
- An index outside an array stops the program with a bounds-check error.

## Operators

From loosest to tightest:

| Operators | Meaning |
| :--- | :--- |
| `\|\|` | logical or |
| `&&` | logical and |
| `==` `!=` `<` `<=` `>` `>=` | comparison |
| `+` `-` `\|` `~` | sum, bitwise or, bitwise xor |
| `*` `/` `%` `%%` `&` `&~` `<<` `>>` | product, remainders, bitwise and, and-not, shifts |
| `-` `!` `~` `&` | negation, logical not, bitwise not, address |
| `a[i]` `p^` | index, dereference |

- `!` is logical not and `~` bitwise not on one operand; between two operands, `~` is bitwise xor.
- Comparisons do not chain: write `a < b && b < c`.
- `x & mask == 0` means `(x & mask) == 0`, since `&` binds tighter than comparisons.
- Keep a shift amount below the type's width.

## Control flow

```odin
package demo

count_down :: proc(n: u32) -> u32 {
	i := n
	steps: u32 = 0
	for i > 0 {
		i -= 1
		if i == 3 {
			continue
		}
		steps += 1
	}
	return steps
}

safe_div :: proc(a: i32, b: i32) -> i32 {
	if b == 0 {
		return 0
	}
	return a / b
}

sign :: proc(x: i32) -> i32 {
	if x < 0 {
		return -1
	} else if x > 0 {
		return 1
	}
	return 0
}

sum_to :: proc(n: int) -> int {
	total := 0
	for i := 1; i <= n; i += 1 {
		total += i
	}
	for i in 0..<3 {
		total += i
	}
	return total
}
```

- `if c { ... } else if d { ... } else { ... }` is a statement. A value that depends on a condition is `a if c else b`, or `c ? a : b`.
- `for` is the only loop:
  - `for c { ... }` repeats while `c` holds;
  - `for i := 0; i < n; i += 1 { ... }` counts;
  - `for i in 0..<n { ... }` runs `i` from `0` to `n - 1`, and `0..=n` includes `n`;
  - `for x in a { ... }` visits the elements of an array, and `for x, i in a { ... }` their indexes too;
  - `for { ... }` repeats until a `break`.
- `break` leaves the innermost loop, and `continue` starts its next round.
- `switch x { case 1: ... case 2, 3: ... case: ... }` picks the first matching case. Cases do not fall through, and `case:` is the default.

## Procedures and pointers

```odin
package demo

swap :: proc(a: ^[4]i32, i: int, j: int) {
	t := a^[i]
	a^[i] = a^[j]
	a^[j] = t
}

demo :: proc() -> i32 {
	v := [4]i32{1, 2, 3, 4}
	swap(&v, 0, 3)
	return v[0]
}
```

- Parameters are passed by value, and an array parameter is a copy.
- A procedure that must change its caller's variable takes a pointer, `^T`, and the caller passes the variable's address, `&v`.
- Through the pointer, `a^` is the variable itself and `a^[i]` an element of the array it points to.
- A task's signature that says a parameter is changed gives it a pointer type.
- A procedure can return several values: `proc(x: int) -> (int, bool)`, returned as `return q, ok`. The tasks need only one.

## Conditions a task states

A task's signature may state what a caller guarantees and what the result satisfies. Odin has no contracts, so they come as comments above the procedure: `// requires lo <= hi` and `// ensures lo <= result && result <= hi`. Callers keep the `requires`; your procedure must make the `ensures` true. A range written in a parameter's comment is a guarantee too. Use `assert(cond)` to check a fact you rely on: it stops the program when the fact is false.

## Unfinished and impossible cases

- `panic("message")` stops the program with a message. Use it for a case that cannot happen, never as an answer that looks right.
- Leave no unfinished code: a stub that returns an invented value fails the task's tests.

## Habits from other languages that Odin rejects

```text
let x = 1;                // x := 1
int x = 1;                // x: int = 1
fn f(a: i32) -> i32 {}    // f :: proc(a: i32) -> i32 { ... }
while (c) { }             // for c { }
for (i = 0; i < n; i++)   // for i := 0; i < n; i += 1 { }
x as u8, (u8)x            // u8(x)
a ^ b                     // a ~ b (xor)
f(&mut v)                 // f(&v), with a ^T parameter
```
