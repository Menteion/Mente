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

1. **Lexical structure**:
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
   - `Implemented` **Tokens**: the lexer (`lex` in `compiler/mente-syntax`) reads every form above, the keywords and reserved words below, and the operators and punctuation `+ - * / % & | ^ ~ << >>`, `== != < <= > >=`, `= += -= *= /= %= &= |= ^= <<= >>=`, `( ) [ ] { } , : . .. ..= ->`, `@`, and `;`, which appears only in an array repeat. An integer literal keeps its value, up to `u128`, and the text it was written with.
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_lexer_reads_every_m1_literal_and_operator" min="1" reason="Every literal form and operator of M1 lexes, and columns count characters" -->
   - `Implemented` **Lexical errors**: these are errors, and lexing goes on after each, so every one is reported:
     - a character outside comments and string literals that Mente does not use, such as `#`, `$`, `'` or a letter outside ASCII (`E0105`);
     - a string literal that a newline or the end of the file interrupts (`E0106`);
     - an escape other than the four above (`E0107`);
     - an integer literal with no digit after its prefix, a `_` that is not between two digits or before its suffix, a digit its base does not have, a suffix written without its `_`, or a value beyond `u128` (`E0108`). A suffix without its `_` has a machine-applicable fix that inserts it.
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_lexical_errors_are_diagnostics_and_lexing_goes_on" min="1" reason="Each lexical error has its code, lexing goes on, and the suffix fix applies" -->
   - `Specified` The keywords M1 uses are `module`, `pub`, `fn`, `mut`, `val`, `var`, `if`, `else`, `while`, `break`, `continue`, `guard`, `return`, `requires`, `ensures`, `decreases`, `invariant`, `and`, `or`, `not`, `true` and `false`.
   - `Specified` These words are reserved for later milestones and cannot be names: `struct`, `bitstruct`, `enum`, `trait`, `type`, `unit`, `dyn`, `own`, `in`, `import`, `extern`, `const`, `match`, `for`, `parallel`, `parallel_for`, `actor`, `with`, `as`, `comptime`, `raise`, `catch`, `is` and `none`. `from` is a keyword only right after a return type, and `todo` is a built-in function name.
2. `Implemented` **Where a newline ends a statement**:
   <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_a_newline_ends_a_statement_where_the_grammar_says" min="1" reason="Newlines end statements, and do not after a token that cannot end one or inside brackets" -->
   <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_clauses_and_expression_bodies_continue_past_a_newline" min="1" reason="Contract clauses and expression bodies go on past a newline where the grammar says" -->
   - Inside parentheses and brackets, a newline is whitespace. Inside a block's braces, a newline ends the statement before it.
   - A newline does not end a statement after a token that cannot end one: a binary operator, `not`, `=`, a compound assignment, `,`, `.`, `->`, `(`, `[` or `{`.
   - A newline does not end a statement before a line that starts with `{`, `else`, `invariant`, `decreases`, `requires` or `ensures`, so `else` may begin its own line, and loop and function clauses may each take a line.
   - Between declarations, newlines are whitespace. A contract clause (`requires`, `ensures`, `decreases`) ends at the end of its line unless the line ends in a token that cannot end it.
3. `Implemented` **A module and its declarations**:
   <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_only_declarations_stand_at_the_top_level" min="1" reason="Anything but a declaration at the top level is an error, a later milestone's with its milestone" -->
   ```ebnf
   module      = { doc comment } [ "module" path ] { item } ;
   path        = identifier { "." identifier } ;
   item        = { doc comment } { attribute } [ "pub" ] function ;
   attribute   = "@" identifier [ "(" <balanced tokens> ")" ] ;
   ```
   - A module may also declare the forms other spec files govern: `struct` and its `@layout` and `@bytes` (`morphic-layouts.md`), `bitstruct` and `@bits`, `enum`, `trait` and `unit`, and `@example` blocks. Today's declaration parser reads them; they are outside M1's calculus. Anything else at the top level is an error: `import`, `extern` and `actor` with the milestone that brings them (`E0109`), and any other token as not a declaration (`E0111`).
4. `Implemented` **Functions**:
   <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_contracts_and_parameters_of_a_header" min="1" reason="A header's parameters, result type and contracts parse, and a missing body is reported" -->
   <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_parse_functions_reads_each_function_with_a_body" min="1" reason="Every function with a body is parsed, and one without a body is not" -->
   ```ebnf
   function    = "fn" identifier "(" [ parameter { "," parameter } [ "," ] ] ")"
                 [ "->" type ] { contract } ( block | "=" expression ) ;
   parameter   = [ "mut" ] identifier ":" type ;
   contract    = ( "requires" | "ensures" | "decreases" ) expression ;
   ```
   - A function header may also carry what later milestones use, which today's declaration parser already reads: type parameters (`<T>`), an error type (`-> T ! E`), a `from` clause, a `with` clause, and the parameter modes `own` and `in`. In a function with a body, each is reported as a construct of a later milestone (`E0109`).
   - A function without a body is a declaration, which the declaration checks of its own spec file read, such as the `from` check of `regions-and-pools.md`; the body parser reads only functions with a body ([`example-bodies`](../docs/design-review.md#example-bodies)).
5. `Implemented` **Types**:
   <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_types_m1_lacks_and_malformed_ones_are_reported" min="1" reason="A type M1 lacks or a malformed one is reported, and a later milestone's type names its milestone" -->
   ```ebnf
   type          = integer type [ refinement ] | "bool" | "[" integer literal "]" type ;
   integer type  = "i8" | "i16" | "i32" | "i64" | "i128" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize" ;
   refinement    = "[" bound ( ".." | "..=" ) bound "]" ;
   bound         = [ "-" ] integer literal | identifier [ ".len" ] ;
   ```
   - A refinement binds to the integer type it follows: `[4]u8[0..10]` is an array of four `u8[0..10]` values.
6. `Implemented` **Statements**:
   <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_statements_of_m1_parse" min="1" reason="Every statement of M1 parses into its canonical form" -->
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
7. `Implemented` **Expressions**:
   <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_operators_bind_and_group_as_the_grammar_says" min="1" reason="Each operator binds and groups as the precedence list says" -->
   <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_chained_comparisons_places_and_if_operands_are_errors" min="1" reason="Chained comparisons, non-places and if operands are errors, with their fixes" -->
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
8. **What the parser reports**:
   - `Implemented` **Forms M1 does not accept**: each is reported with a diagnostic of its own: an expression expected (`E0110`), a token the grammar requires (`E0111`), two statements on one line, with a fix that starts the second on a new line or removes a `;` (`E0112`), a chained comparison (`E0113`), an assignment to what is not a place (`E0114`), an `if` used as an operand, with a fix that adds parentheses (`E0115`), a type M1 lacks or a malformed one (`E0116`), an array repeat whose count is not a literal (`E0117`), and a keyword where a name belongs (`E0101`). A character the lexer does not know is an error (item 1).
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_parse_errors_have_their_codes_and_recovery_reports_one" min="1" reason="Each parse error has its code" -->
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_statements_share_no_line_and_semicolons_get_a_fix" min="1" reason="Two statements on a line and a semicolon are reported, with fixes that apply" -->
   - `Implemented` **Constructs of later milestones**: a construct outside M1's calculus (`core-calculus.md`, item 1) is reported as such (`E0109`), naming the milestone that brings it, or saying that no milestone has it yet, as for `for` loops and the pipe `|>`.
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_constructs_of_later_milestones_name_their_milestone" min="1" reason="Each construct of a later milestone names its milestone" -->
   - `Implemented` **Recovery**: after an error the parser skips to the next statement, past any brackets the statement opened, or in a header to its next clause, so one mistake yields one diagnostic.
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_parse_errors_have_their_codes_and_recovery_reports_one" min="1" reason="One mistake yields one diagnostic, and the next statement parses" -->
   - `Specified` **Other languages' idioms**: each idiom of `surface-syntax.md`, item 8, is reported with a diagnostic of its own.

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
