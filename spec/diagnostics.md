---
title: Structured Diagnostics, Fixes and the Error-Code Catalogue
version: 0.1.0
status: active
date: 2026-11-08
---

# Structured Diagnostics, Fixes and the Error-Code Catalogue

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-mir`, `compiler/mente-cli`

Agents act on what a compiler says, so every problem `mn` reports is data first and prose second: a stable code, a place in the source and, where one exists, an edit that fixes it. Decided in [`diagnostics-as-api`](../docs/design-review.md#diagnostics-as-api).

## 1. Specification

1. `Implemented` **Every problem is a diagnostic** (`compiler/mente-syntax/src/diagnostics.rs`): a code from the catalogue, a severity, a message, a primary span, any related spans that explain it, and any fixes. A span is a half-open byte range of the module's source; its line and column are computed from the source, both 1-based, the column counting characters rather than bytes. Parse errors and every check of `mente-sema` report this way, and declarations, fields and attributes keep their spans so a diagnostic points at the text it is about.
   <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_parse_errors_are_diagnostics_with_spans" min="1" reason="Parse errors carry their code and the span of the offending token" -->
   <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_declarations_keep_their_spans" min="1" reason="Declarations, fields and attributes keep their spans" -->
   <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_a_type_span_covers_every_token_of_the_type" min="1" reason="A field type span covers every token of the type" -->
   <!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_diagnostics_point_at_their_source" min="1" reason="Each check points at the attribute, field, type or name it is about" -->
   <!-- @assert-count target="compiler/mente-syntax/src/diagnostics.rs" symbol="fn test_line_col_counts_lines_and_characters_from_one" min="1" reason="Lines and columns are 1-based and count characters" -->
   - `Specified`: the severities `warning` and `note`. Every diagnostic reported today is an `error`.
2. `Implemented` **The text form**, for people: `error[E0413]: message`, then `--> file:line:column`, the source line with the primary span underlined, each related span as a note, and each fix with its applicability.
   <!-- @assert-count target="compiler/mente-syntax/src/diagnostics.rs" symbol="fn test_text_form_underlines_the_span" min="1" reason="The text form shows code, place, underlined source, notes and fixes" -->
   <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_check_text_shows_code_place_and_fix" min="1" reason="mn check prints the text form" -->
3. `Implemented` **The JSON form**, for tools: `mn check --json` prints one document on standard output and nothing else, `{"version": 1, "diagnostics": [...]}`. A diagnostic has `code`, `severity`, `message`, `primary`, `related` (each a `message` and a `span`) and `fixes` (each a `message`, an `applicability` and `edits`, each edit a `span` and a `replacement`). A span has `file`, `start` and `end` (bytes, half-open), `line` and `column`. The JSON is written by hand: the front-end crates take no dependencies.
   <!-- @assert-count target="compiler/mente-syntax/src/diagnostics.rs" symbol="fn test_json_form_of_a_diagnostic" min="1" reason="The JSON form, byte for byte" -->
   <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_check_json_reports_the_versioned_form" min="1" reason="mn check --json prints only the versioned document" -->
   <!-- @assert-count target="compiler/mente-cli/tests/reproducible.rs" symbol="fn test_diagnostics_are_reproducible" min="1" reason="Every conformance program's JSON is the same on every run and platform, and matches its golden file" -->
   - `Specified` **Versioning**: `version` rises when a field changes meaning or is removed. Adding a field leaves it unchanged, so a reader ignores fields it does not know.
4. `Implemented` **Fixes**: a fix is a set of text edits, each a span and its replacement, marked `machine-applicable` when it can be applied without a person reading it and `suggestion` when it needs judgement. `apply_fix` applies a fix's edits and refuses edits that overlap. A fix is machine-applicable only when it is certain: today that is `E0412`'s, since contiguity and the type's size leave exactly one valid `@bytes` range. Every suggestion a check offers is tested to repair its module.
   <!-- @assert-count target="compiler/mente-syntax/src/diagnostics.rs" symbol="fn test_apply_fix_edits_in_order_and_rejects_overlap" min="1" reason="Edits apply in order, and overlapping ones are refused" -->
   <!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_machine_applicable_fix_gives_the_only_valid_range" min="1" reason="The machine-applicable fix is applied and the module checked again" -->
   <!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_suggested_fixes_repair_the_module" min="1" reason="Each suggestion, applied, repairs its module" -->
   <!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_fixes_are_offered_only_when_they_fit" min="1" reason="A fix is offered only when one fits" -->
   <!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_gap_and_overlap_are_told_apart" min="1" reason="A gap and an overlap get their own wording" -->
   - `Specified`: the valuation Tier 1 cannot rule out and its one-line `guard` prescription (`ghost-contracts-and-totality.md`, Guardrail #1) are reported as a diagnostic whose fix, with applicability `suggestion`, inserts the `guard`.
5. `Implemented` **The catalogue and `mn explain`**: every code has an entry in `compiler/mente-cli/src/catalogue.rs` with a title, an explanation and its governing spec, and, for a code the compiler reports, a program that must fail with it and a near miss that must pass; a test runs both. `mn explain E0413` prints an entry, and `mn explain` lists every code.
   <!-- @assert-count target="compiler/mente-cli/src/catalogue.rs" symbol="fn test_each_reported_code_fails_its_program_and_passes_its_near_miss" min="1" reason="Each reported code fails its program and passes its near miss" -->
   <!-- @assert-count target="compiler/mente-cli/src/catalogue.rs" symbol="fn test_explain_text_shows_the_programs_of_reported_codes" min="1" reason="The explain text shows the two programs" -->
   <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_explain_prints_the_catalogue" min="1" reason="mn explain prints an entry, lists them all, and rejects an unknown code" -->
   <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_help_lists_every_command" min="1" reason="mn --help lists every command, mn explain included" -->
6. `Implemented` **Codes are stable**: a code is never reused. A code a spec defines but no check reports yet is reserved in the catalogue, and a withdrawn code stays there as retired. The catalogue's reported codes are exactly the codes the compiler's sources spell, and every code a document names has an entry.
   <!-- @assert-count target="compiler/mente-cli/src/catalogue.rs" symbol="fn test_catalogue_codes_are_unique_and_in_order" min="1" reason="Codes are unique and in order, and each names an existing spec" -->
   <!-- @assert-count target="compiler/mente-cli/src/catalogue.rs" symbol="fn test_catalogue_covers_every_code_in_the_compiler_and_the_documents" min="1" reason="The catalogue matches the compiler's codes and covers the documents'" -->
   - `Specified` **Ranges**: `E01xx` syntax, `E03xx` provenance and regions, `E04xx` hardware contracts and layouts, `E05xx` contracts and termination, `E06xx` units, `E07xx` `comptime` and FFI. A new code takes the next free number in its range.
7. `Implemented` **MIR reports a rule it breaks as a value**: `MirRegionSoA::verify_branchless` returns a `MirViolation` with the code, the node and its source line, which `mn` prints as `error[E0401]`.
   <!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="fn test_branchless_region_rejects_conditional_jump" min="1" reason="A conditional jump in a @branchless region is reported with its code, node and line" -->
   - `Specified`: once `.mn` code is lowered to MIR (M1), MIR's diagnostics carry source spans like every other.
8. `Implemented` **Every code has a failure class** ([`ai-coding-problems`](../docs/design-review.md#ai-coding-problems)): each catalogue entry carries one class, which `mn explain` prints, so that the agent benchmark classifies a program that fails to build by the codes it reports and counts the failures the re-planning triggers name. Today's classes are syntax, provenance, hardware contract, proof, type (units included) and comptime, following the ranges above.
   <!-- @assert-count target="compiler/mente-cli/src/catalogue.rs" symbol="fn test_every_code_has_a_failure_class" min="1" reason="Every code has its class, and mn explain prints it" -->
   - `Specified`: foreign idiom and unknown name join as classes with their first codes in M1, and effect in M4. A code whose class differs from its range's is listed in the test with its class.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Error strings with a code prefix (`E0413 [Layout Error]: ...`)** | What the compiler had before. A tool had to parse prose to find the code, and there was no span to point at and no fix to apply. An agent then guesses where the problem is and what to change. |
| **The Language Server Protocol's diagnostic as the only form** | LSP counts lines from 0 and columns in UTF-16 code units, and has no applicability levels for fixes. Byte ranges with 1-based lines and character columns are what the text form prints and what an agent edits with; an LSP server can translate them. |
| **A JSON library for the JSON form** | The front-end crates take no dependencies (`CLAUDE.md`). The form is small, and its test pins every byte. |
| **The catalogue as a Markdown table** | `mn explain` and the tests read the same entries. A table in a document would drift from the code that reports the codes, and could not be run. |
| **Reusing a withdrawn code** | Tools, documents and agents' memories keyed on a code would silently change meaning. A retired code costs one entry. |
| **Marking every fix machine-applicable** | A fix applied without reading must not change what the program means. Most layout fixes choose between real options, such as moving a field, adding padding or `@packed`, so they are suggestions; only a fix with one possible outcome is machine-applicable. |

## 3. Verification assertions

<!-- @assert-present file="compiler/mente-syntax/src/diagnostics.rs,compiler/mente-cli/src/catalogue.rs" reason="The diagnostic type and the catalogue exist" -->
<!-- @assert-count target="compiler/mente-syntax/src/diagnostics.rs" symbol="pub const JSON_VERSION: u32 = 1;" min="1" reason="The JSON form is at version 1" -->
