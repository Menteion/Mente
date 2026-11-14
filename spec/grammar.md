---
title: The Grammar of Mente's Surface Syntax
version: 0.2.0
status: active
date: 2026-11-11
---

# The Grammar of Mente's Surface Syntax

**Governs:** `compiler/mente-syntax`

This file is the normative grammar of the language `core-calculus.md` defines ([`grammar-home`](../docs/design-review.md#grammar-home)). The body parser follows it; `surface-syntax.md` gives the reasons for the syntax, and `core-calculus.md` gives the meaning of each form. It grows with each milestone: items 1 to 8 are M1's grammar, which the parser implements, and items 9 to 13 extend it to M2's, written in M2's design closure ([`m2-design-closure`](../docs/design-review.md#m2-design-closure)).

**Notation.** The grammar is EBNF: `rule = ... ;` defines a rule, `|` separates choices, `[ x ]` is optional, `{ x }` repeats zero or more times, `( ... )` groups, and terminals are quoted. Names in angle brackets, such as `<newline>`, are described in words. In M2's items, `rule += ... ;` adds choices to an M1 rule, and `rule = ... ;` replaces it.

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
   - `Specified` These words are reserved for later milestones and cannot be names: `struct`, `bitstruct`, `enum`, `trait`, `type`, `unit`, `dyn`, `own`, `in`, `import`, `extern`, `const`, `match`, `for`, `parallel`, `parallel_for`, `actor`, `with`, `as`, `comptime`, `raise`, `catch`, `is` and `none`. `from` is a keyword only right after a return type, and `todo` and `unreachable` are built-in function names.
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
   - A module may also declare the forms other spec files govern: a `struct` with `@layout` and `@bytes` (`morphic-layouts.md`), `bitstruct` and `@bits`, `enum`, `trait` and `unit`, and `@example` blocks. Today's declaration parser reads them; they are outside the calculus the compiler implements. A `struct` without `@layout` is M2's (item 10). Anything else at the top level is an error: `import`, `extern` and `actor` with the milestone that brings them (`E0109`), and any other token as not a declaration (`E0111`).
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
   - `Implemented`: the declaration parser finds where a return type ends by its brackets, so they must close in order, in a declaration as in a function with a body. A `]` or `>` that closes nothing, and a `[` or `<` never closed, make the return type malformed (`E0116`); the scan would otherwise run past the body, or past the items after it. So does `->` with no type after it.
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_a_return_types_brackets_must_close_in_order" min="1" reason="A return type whose bracket closes nothing or is never closed is E0116, and well-formed ones still parse" -->
   - `Implemented`: a contract's brackets close in order too, or the header is malformed (`E0111`). Inside a bracket a `{` opens a block; outside every bracket it begins the body.
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_a_contracts_brackets_must_close_in_order" min="1" reason="A contract whose bracket another closes, or is never closed, is E0111, and a block inside a bracket stays in the contract" -->
   - `Implemented`: a function's body that the file ends inside has no closing `}` (`E0103`), as a declaration's does. Read as a function without a body, it would be a declaration no one checks.
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_unclosed_declarations_are_rejected" min="1" reason="A declaration or a function body the file ends inside is E0103, naming what it opened" -->
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
   <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_statements_of_m1_parse" min="1" reason="Every statement of M1 parses into its canonical form, the discard included" -->
   ```ebnf
   block       = "{" { statement } "}" ;                  (* statements end at newlines *)
   statement   = binding | assignment | discard | while | guard | return | "break" | "continue" | expression ;
   binding     = ( "val" | "var" ) identifier [ ":" type ] "=" expression ;
   assignment  = place ( "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" ) expression ;
   place       = identifier { "[" expression "]" } ;
   discard     = "_" "=" expression ;                     (* a value discarded on purpose *)
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
   - `[v; N]` is an array of `N` copies of `v`, where `N` is an integer literal. In M1, `.len` on an array is the only use of `.`, and `todo("reason")` and `unreachable("reason")` are calls to the built-ins of those names.
8. **What the parser reports**:
   - `Implemented` **Forms M1 does not accept**: each is reported with a diagnostic of its own: an expression expected (`E0110`), a token the grammar requires (`E0111`), two statements on one line, with a fix that starts the second on a new line or removes a `;` (`E0112`), a chained comparison (`E0113`), an assignment to what is not a place (`E0114`), an `if` used as an operand, with a fix that adds parentheses (`E0115`), a type M1 lacks or a malformed one (`E0116`), an array repeat whose count is not a literal (`E0117`), and a keyword where a name belongs (`E0101`). A character the lexer does not know is an error (item 1).
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_parse_errors_have_their_codes_and_recovery_reports_one" min="1" reason="Each parse error has its code" -->
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_statements_share_no_line_and_semicolons_get_a_fix" min="1" reason="Two statements on a line and a semicolon are reported, with fixes that apply" -->
   - `Implemented` **Nesting has a limit**: expressions, blocks and types nest at most 256 levels deep in a function (`E0127`), since the parser and every phase after it recurse over them. The parser stops descending at the limit, and a tree that grows too high without its recursion, such as a long chain of `+`, is measured without recursion before any later phase runs, whether or not the function has another error; `mn` runs its work on a stack large enough for code at the limit.
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_code_nested_past_the_limit_is_rejected" min="1" reason="Every shape of nesting past the limit is E0127, and code within it parses" -->
   - `Implemented` **Constructs of later milestones**: a construct outside M1's calculus (`core-calculus.md`, item 1) is reported as such (`E0109`), naming the milestone that brings it, or saying that no milestone has it yet, as for `for` loops and the pipe `|>`.
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_constructs_of_later_milestones_name_their_milestone" min="1" reason="Each construct of a later milestone names its milestone" -->
   - `Implemented` **Recovery**: after an error the parser skips to the next statement, past any brackets the statement opened, or in a header to its next clause, so one mistake yields one diagnostic.
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_parse_errors_have_their_codes_and_recovery_reports_one" min="1" reason="One mistake yields one diagnostic, and the next statement parses" -->
   - `Implemented` **Other languages' idioms**: each idiom of `surface-syntax.md`, item 8, is reported with a diagnostic of its own.
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_other_languages_idioms_name_the_mente_spelling" min="1" reason="Each idiom has its own code" -->

9. `Specified` **M2's lexical additions** ([`str-text-model`](../docs/design-review.md#str-text-model)):
   ```ebnf
   byte literal     = "b'" ( <an ASCII character but "'", "\" or a newline> | escape ) "'" ;
   escape           = "\" ( '"' | "'" | "\" | "n" | "t" | "r" | "0" | "x" hex digit hex digit ) ;
   string literal   = '"' { <any character but '"', "\" or a newline> | escape } '"' ;
   ```
   - In a string literal `\x` reaches at most `7F`, so that every literal is valid UTF-8; in a byte literal it is any byte. An escape beyond these is `E0107`, and `\x80` and above in a string literal too.
   - M2 uses the reserved words `struct`, `enum`, `match`, `own`, `raise`, `catch` and `none` as keywords, and the tokens `!`, `?`, `?.` and `=>`. `??` stays without a meaning, and the other reserved words stay reserved.
10. `Specified` **M2's declarations** ([`struct-enum-syntax`](../docs/design-review.md#struct-enum-syntax)):
    ```ebnf
    item          = { doc comment } { attribute } [ "pub" ] ( function | struct | enum ) ;
    struct        = "struct" identifier "{" field { field } "}" ;          (* one field per line *)
    field         = { attribute } [ "pub" ] identifier ":" type ;
    enum          = "enum" identifier "{" variant { variant } "}" ;        (* one variant per line *)
    variant       = identifier [ "(" payload field { "," payload field } [ "," ] ")" ] ;
    payload field = identifier ":" type ;
    ```
    - Fields and variants end at a newline, as statements do. A comma after one is an error whose fix removes it; inside a variant's parentheses, commas separate the payload's fields.
    - A struct has at least one field. A field has no default (`core-calculus.md`, item 13): `= e` after a field's type is an error. Type parameters after a struct's or an enum's name, and an `invariant` block, are constructs of later milestones (`E0109`).
    - `Implemented`: the body parser reads each struct without `@layout` under this grammar, one field per line, an attribute or a doc comment above a field read and left aside. A struct without a field, or a field without its `:` and type, is `E0111`; two fields on one line are `E0112`; a comma after a field is `E0128`, whose machine-applicable fix removes it; type parameters, an `invariant` block, a field's default and a type of a later milestone are `E0109`.
      <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_struct_declarations_take_one_field_per_line" min="1" reason="A struct's fields stand one per line, and each form M2 lacks is reported" -->
    - `Implemented`: the body parser reads each enum under this grammar, one variant per line, a payload's fields named inside its parentheses and separated by commas, across lines too, an attribute or a doc comment above a variant read and left aside. An enum without a variant, or a payload's field without its `:` and type, is `E0111`; two variants on one line are `E0112`; a comma after a variant is `E0128`, whose machine-applicable fix removes it; a payload of unnamed types, Rust's tuple variant, and one in braces, Rust's struct variant, are `E0130`, whose fix names the fields `value`, or `value0`, `value1` and on, or writes parentheses, the next line read as the next variant's; a payload without a field, `A()`, is `E0111`, whose machine-applicable fix removes the parentheses; type parameters and a variant's `= value` are `E0109`. The declaration parser keeps a payload's braces within its variant, and reads each variant in one form, its attributes, its name and its fields' names and types, whatever its spacing, line breaks, comments and trailing commas.
      <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_enum_declarations_take_one_variant_per_line" min="1" reason="An enum's variants stand one per line, and each form M2 lacks is reported" -->
      <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_enum_variants_are_parsed" min="1" reason="The declaration parser reads each variant in one form and keeps a payload's braces in its variant" -->
11. `Specified` **M2's types and signatures**:
    ```ebnf
    type          += "?" type | "[" "]" type | "str" | "String" | "List" "<" type ">" | identifier ;
    result        = type [ "!" identifier ] | "!" identifier ;
    function      = "fn" identifier "(" [ parameter { "," parameter } [ "," ] ] ")"
                    [ "->" result [ "from" identifier { "," identifier } ] ] { contract } ( block | "=" expression ) ;
    parameter     = [ "mut" | "own" ] identifier ":" type ;
    ```
    - `?` and `[]` bind to the type after them: `?[]u8` is an optional view, and `[]?u8` a view of optional bytes. An identifier names a struct or an enum of the module.
    - `Implemented`: an identifier that is no integer type, `bool` or a type of a later milestone is a named type, which the checker finds among the module's structs; a fixed-point type such as `q16.16` and a type argument list are reported as constructs of later milestones (`E0109`).
      <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_struct_declarations_take_one_field_per_line" min="1" reason="A struct's name is a type, and q16.16 and type arguments are later constructs" -->
    - `!` in a result is the error union (`core-calculus.md`, item 16): `-> u32 ! ParseError`, and `-> ! IoError` for a function that gives no value.
12. `Specified` **M2's statements and expressions** ([`match-patterns`](../docs/design-review.md#match-patterns), [`optional-semantics`](../docs/design-review.md#optional-semantics), [`error-union-semantics`](../docs/design-review.md#error-union-semantics), [`slice-ranges`](../docs/design-review.md#slice-ranges), [`exclusivity-projections`](../docs/design-review.md#exclusivity-projections)):
    ```ebnf
    guard         = "guard" ( expression | "val" identifier "=" expression ) "else" block ;
    if expression = "if" ( expression | "val" identifier "=" expression ) block [ "else" ( block | if expression ) ] ;
    discard       = "_" "=" [ "own" ] expression ;
    statement     += ( "val" | "var" ) identifier [ ":" type ] "=" "own" expression    (* a binding that moves *)
                   | place "=" "own" expression ;                                       (* an assignment that moves *)
    place         = identifier { "[" expression "]" | "." identifier } ;
    expression    += match | raise ;
    match         = "match" expression "{" arm { arm } "}" ;                (* one arm per line *)
    arm           = pattern "=>" ( expression | block ) ;
    pattern       = identifier "." identifier [ "(" payload binding { "," payload binding } [ "," ] ")" ]
                  | [ "-" ] integer literal | "true" | "false" | "none" | identifier | "_" ;
    payload binding = identifier ":" ( identifier | "_" ) ;
    raise         = "raise" expression ;
    unary         += "mut" postfix ;                                         (* a call that changes its receiver *)
    postfix       = primary { "[" ( expression | range ) "]" | "." identifier [ arguments ]
                            | "?." identifier | "!" | "catch" "{" arm { arm } "}" } ;
    range         = ".." | expression ".." expression ;
    primary       += "none" | string literal | byte literal ;
    call          = identifier arguments ;
    arguments     = "(" [ argument { "," argument } [ "," ] ] ")" ;
    argument      = [ "mut" | "own" ] expression | identifier ":" [ "own" ] expression ;
    array         = "[" [ element { "," element } [ "," ] ] "]" | "[" expression ";" expression "]" ;
    element       = [ "own" ] expression ;
    ```
    - `own` stands in the six positions `core-calculus.md` gives it (item 12; [`value-copy-cost`](../docs/design-review.md#value-copy-cost)): before a binding's initializer, the value of a plain assignment, a positional or a named argument, an element of an array literal, and a discard's value. What follows it is a whole local binding, an `own` parameter or the whole error a `catch` arm binds (item 20 there). A compound assignment, an array repeat, a returned value, and the value of a branch or an arm, the last expression of its block included, take none.
    - Arguments are positional or named, never both in one list. A named list builds a struct, `Pair(i: 0, j: 1)`, or a variant's payload, `Shape.Rect(w: 3, h: 4)`; a function takes positional arguments.
    - `Implemented`: a call whose first argument is named builds a struct and one whose first is not calls a function, and a list that mixes the two is `E0111`; `a.name` reads a field; a place is a name followed by indexes and fields, after `mut` in an argument too.
      <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_structs_are_built_with_named_arguments_and_fields_are_places" min="1" reason="Named lists build structs, and places hold fields" --> The shapes `a.b` and `a.b(...)` are a field, a variant or a call on a receiver as name resolution finds `a`: a binding, an enum, or `List` and `String`, whose static calls are `List.new()`, `List.from(v)`, `String.new()` and `String.from(s)`. No binding is named like a type of its module, so `a` is never both.
    - `mut` before a postfix that ends in a call marks a call that changes its receiver: `mut xs.push(v)`, and `val last = mut xs.pop()`. After `mut` in an argument, a postfix that ends in a call is such a call, and any other is a place.
    - `!` and `catch` follow a call. `!` binds as a postfix operator, so `f(x)! + 1` adds to the value. The lexer reads `!=` as one token, so `f()!= 3` is a comparison; `f()! == 3` compares the value.
    - `match` and `raise` stand alone as full expressions, as `if` does; inside an operator's operand they need parentheses.
    - Arms end at a newline, as statements do, and a comma after one is an error whose fix removes it. A newline after `=>` does not end the arm, and a line that starts with `catch` continues the call before it, as one that starts with `else` continues an `if`.
    - `Implemented`: the body parser reads `match` under this grammar, one arm per line, an arm an expression or a block, and a pattern a variant with its bindings, an integer literal with `-` before it or not, `true`, `false` or `_`. A `match` inside an operator's operand, or as a `while`'s condition, is `E0115`, whose fix adds the parentheses; a `match` without an arm or its `}`, and an arm without `=>`, are `E0111`, and so is an arm that holds an assignment, whose fix writes it in a block, the next line read as the next arm; two arms on one line are `E0112`; a comma after an arm is `E0128`, whose machine-applicable fix removes it; `none` and a name as a pattern, the patterns of `?T`, are `E0109`. `a.b(...)` builds a variant where `a` is not a binding, and the same call on a type of the language, such as `List.new()`, is `E0109`; after an error among a call's arguments, the parser resumes after its `)`.
      <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_match_arms_and_patterns_parse" min="1" reason="Arms stand one per line, patterns and variants parse, and each misplaced form is reported" -->
13. `Specified` **What the parser reports in M2**: each form of another language that M2's constructs meet is reported with a fix to Mente's spelling (`surface-syntax.md`, item 8): Rust's struct literal `Name { f: e }`, to `Name(f: e)`; Rust's `?` after a call, to `!`; `if let` and `guard let`, to `if val` and `guard val`; `Some(x)` and `None`, to `x` and `none`; `Vec::new()` and `vec![]`, to `List.new()`; a positional payload in a variant's declaration, to named fields; and a comma after a field, a variant or an arm, removed.
    - `Implemented`: Rust's struct literal `Name { f: e }` is `E0129`, read as `Name(f: e)` after it, whose machine-applicable fix writes the parentheses; after a condition, `{` opens the block that follows. A comma after a field is `E0128` (item 10).
      <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_a_struct_literal_in_braces_gets_its_fix" min="1" reason="A struct literal in braces is E0129 with a fix that applies" -->
    - `Implemented`: a variant built in braces, `Name.B { f: e }`, is `E0129` too, read as `Name.B(f: e)` after it, whose machine-applicable fix writes the parentheses.
      <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_match_arms_and_patterns_parse" min="1" reason="A variant built in braces is E0129 with a fix that applies" -->
    - `Implemented`: a payload of unnamed types, or one in braces, is `E0130` (item 10); an arm's guard, Rust's `pattern if condition =>`, is `E0131`; alternatives `0 | 1`, a range `0..3`, a tuple, an array, and a pattern in a payload's field in place of a name, are `E0132`; a variant written with `::`, `E::A`, is `E0123`; and a comma after an arm is `E0128`, whose fix removes it and after which the arms that follow still parse.
      <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_other_languages_patterns_get_their_codes" min="1" reason="Other languages' arm guards, alternatives, ranges, tuples and nested patterns have their codes" -->

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
| **Commas between fields, variants and match arms (Rust)** | Mente ends a statement at a newline, and a declaration's fields and a `match`'s arms are written one per line alike, so one separator serves all of them. A comma has a fix that removes it, which teaches the rule in one step. |
| **`if let` and `guard let` (Swift), or `if (x != null)` narrowing** | `val` is Mente's word for a binding that never changes, and the present value of a `?T` gets a name of its own (`core-calculus.md`, item 15). |
| **An unmarked call that changes its receiver (Rust's `v.push(x)`, Swift's mutating methods)** | `mut` is written at every call site that changes an argument (`mutable-value-semantics.md`, item 2), and a receiver is an argument. `mut xs.push(v)` shows the change where it happens. |
| **A grammar file for a parser generator** | A generator would add a dependency, which the front-end crates do not take, and a second source of truth beside the hand-written Pratt parser. The EBNF here is for reading; the conformance suite tests the parser against it. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Semi," min="1" reason="The lexer has a ; token, which appears only in the array repeat [v; N]" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol="while lo < hi decreases hi - lo {" min="1" reason="A loop's clauses sit in its header" -->
