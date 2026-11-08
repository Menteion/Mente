---
title: The Grammar of M1's Surface Syntax
version: 0.1.0
status: active
date: 2026-11-08
---

# The Grammar of M1's Surface Syntax

**Governs:** `compiler/mente-syntax`

This file is the normative grammar of the language `core-calculus.md` defines for M1 ([`grammar-home`](../docs/design-review.md#grammar-home)). The body parser follows it; `surface-syntax.md` gives the reasons for the syntax, and `core-calculus.md` gives the meaning of each form. It grows with each milestone.

**Notation.** The grammar is EBNF: `rule = ... ;` defines a rule, `|` separates choices, `[ x ]` is optional, `{ x }` repeats zero or more times, `( ... )` groups, and terminals are quoted. Names in angle brackets, such as `<newline>`, are described in words.

## 1. Specification

1. `Specified` **Lexical structure**:
   ```ebnf
   comment          = "//" { <any character but a newline> } ;
   doc comment      = "///" { <any character but a newline> } ;          (* exactly three slashes *)
   identifier       = ( letter | "_" ) { letter | digit | "_" } ;         (* ASCII in M1 *)
   integer literal  = ( decimal | "0x" hex digits | "0b" binary digits ) [ suffix ] ;
   decimal          = digit { [ "_" ] digit } ;
   hex digits       = hex digit { [ "_" ] hex digit } ;
   binary digits    = ( "0" | "1" ) { [ "_" ] ( "0" | "1" ) } ;
   suffix           = "_" integer type ;                                  (* 255_u8; 0xFF_u8 *)
   string literal   = '"' { <any character but '"', "\" or a newline> | "\" ( '"' | "\" | "n" | "t" ) } '"' ;
   ```
   - The keywords M1 uses are `module`, `pub`, `fn`, `mut`, `val`, `var`, `if`, `else`, `while`, `break`, `continue`, `guard`, `return`, `requires`, `ensures`, `decreases`, `invariant`, `and`, `or`, `not`, `true` and `false`.
   - These words are reserved for later milestones and cannot be names: `struct`, `bitstruct`, `enum`, `trait`, `type`, `unit`, `dyn`, `own`, `in`, `import`, `extern`, `const`, `match`, `for`, `parallel`, `parallel_for`, `actor`, `with`, `as`, `comptime`, `raise`, `catch`, `is` and `none`. `from` is a keyword only right after a return type, and `todo` is a built-in function name.
   - The operators and punctuation are `+ - * / % & | ^ ~ << >>`, `== != < <= > >=`, `= += -= *= /= %= &= |= ^= <<= >>=`, `( ) [ ] { } , : . .. ..= ->`, `@`, and `;`, which appears only in an array repeat.
2. `Specified` **Where a newline ends a statement**:
   - Inside parentheses and brackets, a newline is whitespace. Inside a block's braces, a newline ends the statement before it.
   - A newline does not end a statement after a token that cannot end one: a binary operator, `not`, `=`, a compound assignment, `,`, `.`, `->`, `(`, `[` or `{`.
   - A newline does not end a statement before a line that starts with `{`, `else`, `invariant`, `decreases`, `requires` or `ensures`, so `else` may begin its own line, and loop and function clauses may each take a line.
   - Between declarations, newlines are whitespace. A contract clause (`requires`, `ensures`, `decreases`) ends at the end of its line unless the line ends in a token that cannot end it.
3. `Specified` **A module and its declarations**:
   ```ebnf
   module      = { doc comment } [ "module" path ] { item } ;
   path        = identifier { "." identifier } ;
   item        = { doc comment } { attribute } [ "pub" ] function ;
   attribute   = "@" identifier [ "(" <balanced tokens> ")" ] ;
   ```
   - A module may also declare the forms other spec files govern: `struct` and its `@layout` and `@bytes` (`morphic-layouts.md`), `bitstruct` and `@bits`, `enum`, `trait` and `unit`, and `@example` blocks. Today's declaration parser reads them; they are outside M1's calculus.
4. `Specified` **Functions**:
   ```ebnf
   function    = "fn" identifier "(" [ parameter { "," parameter } [ "," ] ] ")"
                 [ "->" type ] { contract } ( block | "=" expression ) ;
   parameter   = [ "mut" ] identifier ":" type ;
   contract    = ( "requires" | "ensures" | "decreases" ) expression ;
   ```
   - A function header may also carry what later milestones use, which today's declaration parser already reads: type parameters (`<T>`), an error type (`-> T ! E`), a `from` clause, a `with` clause, and the parameter modes `own` and `in`.
5. `Specified` **Types**:
   ```ebnf
   type          = integer type [ refinement ] | "bool" | "[" integer literal "]" type ;
   integer type  = "i8" | "i16" | "i32" | "i64" | "i128" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize" ;
   refinement    = "[" bound ( ".." | "..=" ) bound "]" ;
   bound         = [ "-" ] integer literal | identifier [ ".len" ] ;
   ```
   - A refinement binds to the integer type it follows: `[4]u8[0..10]` is an array of four `u8[0..10]` values.
6. `Specified` **Statements**:
   ```ebnf
   block       = "{" { statement } "}" ;                  (* statements end at newlines *)
   statement   = binding | assignment | while | guard | return | "break" | "continue" | expression ;
   binding     = ( "val" | "var" ) identifier [ ":" type ] "=" expression ;
   assignment  = place ( "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" ) expression ;
   place       = identifier { "[" expression "]" } ;
   while       = "while" expression { loop clause } block ;
   loop clause = ( "invariant" | "decreases" ) expression ;
   guard       = "guard" expression "else" block ;
   return      = "return" [ expression ] ;
   ```
7. `Specified` **Expressions**:
   ```ebnf
   expression     = if expression | or ;
   if expression  = "if" expression block [ "else" ( block | if expression ) ] ;
   or             = and { "or" and } ;
   and            = not { "and" not } ;
   not            = "not" not | comparison ;
   comparison     = bit or [ ( "==" | "!=" | "<" | "<=" | ">" | ">=" ) bit or ] ;
   bit or         = bit xor { "|" bit xor } ;
   bit xor        = bit and { "^" bit and } ;
   bit and        = shift { "&" shift } ;
   shift          = sum { ( "<<" | ">>" ) sum } ;
   sum            = product { ( "+" | "-" ) product } ;
   product        = unary { ( "*" | "/" | "%" ) unary } ;
   unary          = ( "-" | "~" ) unary | postfix ;
   postfix        = primary { "[" expression "]" | "." identifier } ;
   primary        = integer literal | "true" | "false" | call | identifier | "(" expression ")" | array ;
   call           = identifier "(" [ argument { "," argument } [ "," ] ] ")" ;
   argument       = [ "mut" ] expression ;                 (* after mut, a place *)
   array          = "[" [ expression { "," expression } [ "," ] ] "]" | "[" expression ";" expression "]" ;
   ```
   - Precedence rises down the rule list: `or`, `and`, `not`, comparisons, `|`, `^`, `&`, shifts, `+` and `-`, `*`, `/` and `%`, the unary operators, then indexing and `.`. Binary operators group to the left.
   - `not` binds more loosely than a comparison, so `not a == b` is `not (a == b)`; the bit operators bind more tightly than comparisons, so `x & mask == 0` is `(x & mask) == 0`.
   - Comparisons do not chain: `a < b < c` is an error whose fix suggests `a < b and b < c`.
   - An `if` expression stands alone as a full expression, such as an initializer, an argument or a returned value; inside an operator's operand it needs parentheses.
   - `[v; N]` is an array of `N` copies of `v`, where `N` is an integer literal. In M1, `.len` on an array is the only use of `.`, and `todo("reason")` is a call to the built-in `todo`.
8. `Specified` **What the parser reports**:
   - Every form above that M1 does not accept, every construct outside M1's calculus (`core-calculus.md`, item 1), and every idiom of another language (`surface-syntax.md`, item 8) is reported with a diagnostic of its own; a character the lexer does not know is an error.
   - After an error the parser recovers at the next statement or declaration, so one mistake yields one diagnostic.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Semicolons that end statements** | Mente ends a statement at a newline (`surface-syntax.md`, item 1). The newline rules here say exactly where that happens, as Go and Swift do, so an agent never has to guess. |
| **`[v] * N` or a `fill` method for an array of copies** | The lexer was given its `;` token for `[v; N]`, which agents know from Rust. `[v] * N` would give `*` a second meaning on arrays, and a method needs generics, which M1 lacks. |
| **`not` binding tightly, as Rust's `!` does** | Mente took its word operators from Python, where `not a == b` means `not (a == b)`; an agent reading `not` expects Python's precedence. |
| **Bit operators below comparisons, as in C** | C's `x & mask == 0` means `x & (mask == 0)`, a classic bug; Rust and Python bind `&` more tightly than `==`, and so does Mente. |
| **Chained comparisons, as in Python (`a < b < c`)** | Readers from Rust and C would take `a < b < c` as `(a < b) < c`. An error with a one-step fix is clearer than either meaning. |
| **Requiring `else` on the line of the closing brace, as Go does** | Both styles are common in agents' training data; a line that starts with `else` can only continue an `if`, so allowing it costs nothing. |
| **Loop invariants and measures as statements at the top of the body** | They describe the loop as a whole, as `requires` describes a function, so they sit in its header where Tier 1 and `mn view` read them, as example 01's `while lo < hi decreases hi - lo` already does. |
| **A grammar file for a parser generator** | A generator would add a dependency, which the front-end crates do not take, and a second source of truth beside the hand-written Pratt parser. The EBNF here is for reading; the conformance suite tests the parser against it. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Semi," min="1" reason="The lexer has a ; token, which appears only in the array repeat [v; N]" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol="while lo < hi decreases hi - lo {" min="1" reason="A loop's clauses sit in its header" -->
