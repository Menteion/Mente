---
title: Surface Syntax, Symmetric Bindings, and Expression Control Flow
version: 0.1.0
status: active
date: 2026-11-04
---

# Surface Syntax, Symmetric Bindings, and Expression Control Flow

**Governs:** `compiler/mente-syntax`, `compiler/mente-ast`

## 1. Specification

The body parser in `mente-syntax` reads the statements and expressions of M1 (`grammar.md`); the forms below that M1 lacks come with later milestones, and the body parser reports them as such.

1. **Block delimiters and line termination**:
   - `Specified`: blocks are delimited by explicit braces `{` and `}`.
   - `Implemented`: the declaration parser matches a body's braces, so a body ends at the brace that closes its first one.
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_nested_braces_end_at_their_match" min="1" reason="A body with nested braces ends at its matching brace" -->
   - `Specified`: statements terminate at newlines without trailing semicolons `;`. Line continuation occurs automatically after binary operators (`+`, `-`, `*`, `/`, `|>`, `and`, `or`) or inside unclosed parentheses and brackets; inside a block's braces a newline separates statements. `grammar.md`, item 2, gives the exact rules.
2. `Specified` **Symmetric 3-letter variable bindings**:
   - `val name: Type = expr` declares an immutable binding.
   - `var name: Type = expr` declares a local mutable binding.
   - The keywords `let`, `let mut`, and `const` MUST NOT be used for local bindings. The body parser reads `val` and `var` bindings (`grammar.md`, item 6); `let` still lexes as an identifier, and gets its own diagnostic with the idioms of item 8.
3. `Implemented` **Function headers and return syntax** (`parse_fn`):
   <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_function_headers_return_types_and_expression_bodies" min="1" reason="parse_fn reads the return type and tells a block body from an expression body" -->
   - Return types follow `->`: `pub fn clamp(x: i32, hi: i32) -> i32`.
   - Single-expression functions MAY use `=`: `pub fn square(x: i32) -> i32 = x * x`.
4. `Specified` **Expression-oriented branching, `guard`, and pipe `|>`**:
   - `if` and `match` are expressions that return the value of their evaluated branch; Mente has no ternary `? :` operator.
   - `guard <cond> else { <diverging-expr> }` enforces preconditions and narrows types for the rest of the enclosing block.
   - Data transformation chains use the left-to-right pipe operator `x |> f(a) |> g(b)`.
   - The body parser reads `if` and `guard` (`grammar.md`, items 6 and 7); `match` and `|>` come later, and are reported as such.
5. `Specified` **Logical word operators**:
   - Boolean conjunction, disjunction, and negation use the keywords `and`, `or`, and `not`, reserving `&`, `|`, `^`, and `~` exclusively for bitwise operations and type unions (`A | B`). The lexer has `Pipe`, `Amp`, `Caret` and `Tilde` tokens for `|`, `&`, `^` and `~`. It has `And`, `Or` and `Not` tokens.
6. `Specified` **Numeric literals and suffixes**:
   - Digits may be grouped with `_` between digits: `1_000_000`, `0xFF_FF`. In a hexadecimal literal `a`–`f` are digits, so there a suffix starts with `_` and a letter beyond `f` (`0xFF_u8`).
   - A suffix starts with `_` followed by a letter and runs to the end of the identifier. It names a unit in scope (`200.0_meter`, `9.5_sec`, `5_ms`; `dimensional-units.md`), a fixed-point format with the type name's `.` written as `_` so the literal stays one token (`0.5_q16_16` is a `q16.16`, `0.0_q1_15` a `q1.15`), or an integer type (`255_u8`, `5_u4`).
   - A suffix that names none of these is an error; `_q16` alone is not a format. A literal outside its type's range is an error (`300_u8`).
7. `Specified` **An honest unfinished body: `todo("reason")`** ([`todo-marker`](../docs/design-review.md#todo-marker)):
   - `todo("reason")` is an expression of any type that traps when reached, for a body its author could not finish.
   - `mn check` lists every `todo` with its reason, a release build rejects a module that contains one, and a function containing one is never the evidence for an `Implemented` claim.
   - `Implemented`: `mn check` lists every `todo` of a module's function bodies with its reason and its place, after the file's diagnostics and in `--json`; the list changes nothing in the verdict. A release build does not exist yet.
     <!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_listings_name_every_todo_and_trusted_clause" min="1" reason="Every todo is listed with its reason and place" -->
     <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_check_lists_todos_and_trusted_clauses" min="1" reason="mn check prints every todo" -->
8. `Implemented` **Other languages' idioms get errors of their own** ([`foreign-idiom-diagnostics`](../docs/design-review.md#foreign-idiom-diagnostics)):
   <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_other_languages_idioms_name_the_mente_spelling" min="1" reason="Each idiom in a body has its code, its message and its fix" -->
   <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_lifetimes_and_async_are_idioms_wherever_they_stand" min="1" reason="Lifetimes and async are reported wherever they stand, with suggestions" -->
   - The parser recognises each and reports it under its own code, naming the Mente spelling: `let` and `let mut` (`E0118`); `&x` and `&mut x` (`E0119`); lifetimes such as `'a`, anywhere in a module (`E0120`); `async` and `await` (`E0121`); `.unwrap()` and `.expect(...)` (`E0122`); `::` in a path and the turbofish `::<>` (`E0123`); `&&`, `||` and `!` (`E0124`); the conditional `c ? a : b` (`E0125`); and the cast `as` (`E0126`). Where the idiom's meaning is plain, as for `let` and `&&`, the parser reads it as the Mente spelling and checks the rest of the statement.
   - A fix is `machine-applicable` only where the rewrite is unique: `let mut` to `var`, `let` to `val`, a call site's `&mut x` to `mut x`, `::<` to `<`, `&&`, `||` and `!` to `and`, `or` and `not` (with parentheses where `not` would group differently, as in `(not a) == b`), and `c ? a : b` to `if c { a } else { b }`. `.unwrap()` (a `guard` or `??`), `async` and `await` (`with` effects and `parallel (...)`), `<'a>` (a `from` clause or a parameter mode), `&x` elsewhere than a call, a path's `::`, and `as` need judgement and get suggestions.
   - A character the lexer does not know is an error, `E0105` (`grammar.md`, item 1).
   - `Specified`: M2 adds the idioms its constructs meet, each with its fix (`grammar.md`, item 13): Rust's struct literal `Name { f: e }`, `?` after a call, `if let`, `Some(x)` and `None`, and `Vec::new()`, and a comma after a field, a variant or an arm. A prefix `!` stays the idiom for `not`; a `!` after a call is M2's propagation.
   - `Implemented`: the idioms of M2's structs: Rust's struct literal `Name { f: e }` (`E0129`) and a comma after a field (`E0128`), each with a machine-applicable fix.
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_a_struct_literal_in_braces_gets_its_fix" min="1" reason="Rust's struct literal is E0129 with its fix" -->
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_struct_declarations_take_one_field_per_line" min="1" reason="A comma after a field is E0128 with its fix" -->
9. `Specified` **A case that cannot happen: `unreachable("reason")`** ([`unreachable-marker`](../docs/design-review.md#unreachable-marker)):
   - `unreachable("reason")` is an expression of any type that traps when reached, for a case its author knows cannot happen, such as the `else` of a `guard` whose condition a contract implies but Tier 1 cannot prove.
   - Tier 1 erases it where it proves the case impossible. Unlike `todo`, it marks finished code: a release build accepts it, and the agent benchmark scores it like any other code.
   - `Implemented`: the body parser reads `todo("reason")` and `unreachable("reason")` with their string reasons, and reports a reason that is not a string literal (`E0111`).
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_operators_bind_and_group_as_the_grammar_says" min="1" reason="todo and unreachable parse with their reasons" -->

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Significant whitespace indentation (Python / Mojo / Nim)** | Fragile under automated code synthesis, diff patching, and multi-line contract blocks (`requires` / `ensures`). Explicit `{ }` guarantees unambiguous structural boundaries. |
| **Asymmetric `let` / `let mut` (Rust)** | Visual alignment breaks between immutable (3 chars) and mutable (7 chars) bindings. `val` and `var` are symmetric 3-character keywords. |
| **Symbolic boolean operators `&&`, `||`, `!` (C / Rust / Go)** | Prefix `!` collides visually with postfix error propagation `expr!`, and `||` collides with zero-argument closure syntax. `and`, `or`, `not` eliminate both ambiguities. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Val," min="1" reason="Lexer defines Val token for immutable bindings" -->
<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Var," min="1" reason="Lexer defines Var token for mutable bindings" -->
<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Guard," min="1" reason="Lexer defines Guard token for early-exit precondition narrowing" -->
<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="PipeGt," min="1" reason="Lexer defines PipeGt (|>) for left-to-right data pipelines" -->
<!-- @assert-absence target="examples" symbol="let mut" glob="*.mn" reason="Mente uses symmetric val/var bindings and forbids let mut" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="0.75_q16_16" min="1" reason="Example 02 writes q16.16 literals with the full format, dot as underscore" -->
