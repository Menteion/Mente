---
title: Surface Syntax, Symmetric Bindings, and Expression Control Flow
version: 0.1.0
status: active
date: 2026-11-04
---

# Surface Syntax, Symmetric Bindings, and Expression Control Flow

**Governs:** `compiler/mente-syntax`, `compiler/mente-ast`

## 1. Specification

1. **Block delimiters and line termination (`Implemented` in `compiler/mente-syntax/src/lib.rs`)**:
   - Blocks are delimited by explicit braces `{` and `}`.
   - Statements terminate at newlines without trailing semicolons `;`. Line continuation occurs automatically after binary operators (`+`, `-`, `*`, `/`, `|>`, `and`, `or`) or inside unclosed delimiters `(...)`, `[...]`, `{...}`.
2. **Symmetric 3-letter variable bindings (`Implemented`)**:
   - `val name: Type = expr` declares an immutable binding.
   - `var name: Type = expr` declares a local mutable binding.
   - The keywords `let`, `let mut`, and `const` MUST NOT be used for local bindings.
3. **Function headers and return syntax (`Implemented`)**:
   - Return types follow `->`: `pub fn clamp(x: i32, hi: i32) -> i32`.
   - Single-expression functions MAY use `=`: `pub fn square(x: i32) -> i32 = x * x`.
4. **Expression-oriented branching, `guard`, and pipe `|>` (`Implemented`)**:
   - `if` and `match` are expressions that return the value of their evaluated branch; Mente has no ternary `? :` operator.
   - `guard <cond> else { <diverging-expr> }` enforces preconditions and narrows types on the main indentation column.
   - Data transformation chains use the left-to-right pipe operator `x |> f(a) |> g(b)` (`TokenKind::PipeArrow`).
5. **Logical word operators (`Implemented`)**:
   - Boolean conjunction, disjunction, and negation use the keywords `and`, `or`, and `not` (`TokenKind::KwAnd`, `KwOr`, `KwNot`), reserving `&`, `|`, `^`, and `~` exclusively for bitwise operations and type unions (`A | B`).

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Significant whitespace indentation (Python / Mojo / Nim)** | Fragile under automated code synthesis, diff patching, and multi-line contract blocks (`requires` / `ensures`). Explicit `{ }` guarantees unambiguous structural boundaries. |
| **Asymmetric `let` / `let mut` (Rust)** | Visual alignment breaks between immutable (3 chars) and mutable (7 chars) bindings, and `let` conflicts with mathematical set notation vs. MVS `Let` parameter projection. `val` and `var` are symmetric 3-character keywords. |
| **Symbolic boolean operators `&&`, `||`, `!` (C / Rust / Go)** | Prefix `!` collides visually with postfix error propagation `expr!`, and `||` collides with zero-argument closure syntax. `and`, `or`, `not` eliminate both ambiguities. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Val," min="1" reason="Lexer defines Val token for immutable bindings" -->
<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Var," min="1" reason="Lexer defines Var token for mutable bindings" -->
<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Guard," min="1" reason="Lexer defines Guard token for early-exit precondition narrowing" -->
<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="PipeGt," min="1" reason="Lexer defines PipeGt (|>) for left-to-right data pipelines" -->
<!-- @assert-absence target="examples" symbol="let mut" glob="*.mn" reason="Mente uses symmetric val/var bindings and forbids let mut" -->
