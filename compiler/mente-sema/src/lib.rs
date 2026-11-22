//! `mente-sema` — Semantic Prover: 64B Cache-Line Layout, `bitstruct`, `@total`/`decreases`, and Micro-Arch Contracts.
//!
//! Governed by `spec/morphic-layouts.md`, `spec/hardware-contracts.md` and
//! `spec/ghost-contracts-and-totality.md`. The `@total` and `@branchless` checks here are
//! token scans over unparsed bodies, not proofs; see `docs/STATUS.md`. Every problem is a
//! [`Diagnostic`] with its code, its span and, where one is certain or plausible, a fix
//! (`spec/diagnostics.md`).

// A match over an enum names every variant, so that a variant added later does not compile until
// each match decides what it does with it; a `_` arm once let walkers skip expression kinds
// ([`guard-per-problem`](../../docs/design-review.md#guard-per-problem)). A match that is a
// classifier by nature keeps `#[expect]`, with the reason every variant belongs in its arm.
#![deny(clippy::wildcard_enum_match_arm)]

pub mod bounds;
pub mod check;
mod conditions;
pub mod frames;
pub mod incremental;
pub mod layout;
pub mod literals;
pub mod names;
pub mod octagon;
pub mod structs;
pub mod tier1;
pub mod types;
pub mod wide;

use mente_syntax::diagnostics::{in_source_order, Applicability, Diagnostic};
use mente_syntax::{parse_module, FnDecl, ItemDecl, ModuleFile, ParamMode, Span, TokenKind};

use layout::{element_type, layout_attr};
pub use layout::{verify_bitstruct_layout, verify_explicit_struct_layout, LayoutNum};

#[derive(Clone, Debug)]
pub struct LayoutProof {
    pub struct_name: String,
    pub total_bytes: LayoutNum,
    pub align_bytes: LayoutNum,
    pub markdown_table: String,
}

#[derive(Clone, Debug)]
pub struct BitStructProof {
    pub name: String,
    pub total_bits: LayoutNum,
}

#[derive(Clone, Debug)]
pub struct ModuleVerificationReport {
    pub module_path: String,
    pub layout_proofs: Vec<LayoutProof>,
    pub bitstruct_proofs: Vec<BitStructProof>,
    pub total_functions_verified: usize,
    pub branchless_functions_verified: usize,
    pub kernel_functions_verified: usize,
    pub examples_count: usize,
}

/// Parses and checks the source of one module: what `mn check` reports for it.
pub fn check_source(src: &str) -> Result<ModuleVerificationReport, Vec<Diagnostic>> {
    let module = parse_module(src).map_err(|d| vec![d])?;
    check_module(src, &module)
}

/// Checks a parsed module: the declaration checks of [`verify_module`], then every function
/// with a body, under M1's grammar (`spec/grammar.md`), its names and types and Tier 1
/// (`spec/core-calculus.md`). The diagnostics come in the order of the source.
pub fn check_module(
    src: &str,
    module: &ModuleFile,
) -> Result<ModuleVerificationReport, Vec<Diagnostic>> {
    let b = bodies(src, module, true);
    combine(verify_module_with(module, &b.structs), b.errors)
}

/// The declaration checks' verdict and the bodies' diagnostics, in the order of the source.
fn combine<T>(
    verified: Result<T, Vec<Diagnostic>>,
    body_errors: Vec<Diagnostic>,
) -> Result<T, Vec<Diagnostic>> {
    let mut errors = match verified {
        Ok(report) if body_errors.is_empty() => return Ok(report),
        Ok(_) => body_errors,
        Err(mut errors) => {
            errors.extend(body_errors);
            errors
        }
    };
    in_source_order(&mut errors);
    Err(errors)
}

/// For each function whose names and types check, in source order, its name and what Tier 1
/// found: every obligation, proven or left to run time, which `mn view --checks` lists.
pub fn tier1_reports(src: &str) -> Vec<(String, tier1::Report)> {
    match parse_module(src) {
        Ok(module) => bodies(src, &module, true).reports,
        Err(_) => Vec::new(),
    }
}

/// For each function whose names and types check, in source order, its name, what Tier 1 found,
/// and each obligation Tier 1 discharged as an SMT-LIB 2 query for the SMT cross-check, which
/// `mn smt` writes (`tier1::smt`).
pub fn tier1_queries(src: &str) -> Vec<(String, tier1::Report, Vec<tier1::smt::Query>)> {
    match parse_module(src) {
        Ok(module) => {
            let ask = tier1::Ask {
                queries: true,
                replays: false,
            };
            let b = bodies_with(src, &module, true, ask);
            b.reports
                .into_iter()
                .zip(b.extras)
                .map(|((name, report), extras)| (name, report, extras.queries))
                .collect()
        }
        Err(_) => Vec::new(),
    }
}

/// For each function whose names and types check, in source order, its name, how many loops
/// Tier 1 met again and replayed from its memo, each analysed again beside its replay, and the
/// span of each whose replay that analysis found different (`tier1::LoopOutcome`): none, unless
/// the outcome misses something the analysis of a loop changes. The replay test runs it over the
/// corpus and the generated programs.
pub fn tier1_loop_replays(src: &str) -> Vec<(String, usize, Vec<Span>)> {
    match parse_module(src) {
        Ok(module) => {
            let ask = tier1::Ask {
                queries: false,
                replays: true,
            };
            let b = bodies_with(src, &module, true, ask);
            b.reports
                .into_iter()
                .zip(b.extras)
                .map(|((name, _), extras)| {
                    (name, extras.replays_compared, extras.replays_differing)
                })
                .collect()
        }
        Err(_) => Vec::new(),
    }
}

/// What `mn check` lists beside a module's diagnostics: every `todo` with its reason, and
/// every trusted clause, one of an `@prove` function outside the Tier-1 fragment, assumed until
/// `mn prove` runs. Each comes with its place, in source order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listings {
    pub todos: Vec<(Span, String)>,
    pub axioms: Vec<(Span, String)>,
}

pub fn listings(src: &str) -> Listings {
    let Ok(module) = parse_module(src) else {
        return Listings::default();
    };
    let b = bodies(src, &module, true);
    let mut axioms: Vec<(Span, String)> =
        b.reports.into_iter().flat_map(|(_, r)| r.axioms).collect();
    axioms.sort_by_key(|t| t.0.start);
    Listings {
        todos: b.todos,
        axioms,
    }
}

/// A function whose body parsed, with its names resolved and its types checked.
#[derive(Clone, Debug)]
pub struct CheckedFn {
    pub syntax: mente_syntax::tree::FnSyntax,
    pub resolution: names::Resolution,
    pub typing: check::Typing,
    /// Every obligation Tier 1 recorded in the function, proven or not; none when the module
    /// was checked without it.
    pub obligations: Vec<tier1::Obligation>,
    /// How many variables Tier 1 gave the function (`tier1::Report`); 0 when the module was
    /// checked without it.
    pub variables: usize,
}

/// A module that passed every check, with its functions in order: those with a body checked,
/// a declaration without one as `None`.
#[derive(Clone, Debug)]
pub struct CheckedModule {
    pub names: Vec<String>,
    pub functions: Vec<Option<CheckedFn>>,
    /// The module's structs, with their fields and layouts.
    pub structs: structs::Structs,
    /// For each function, as `functions` has them, its frame: the bytes a call of it counts
    /// against the stack budget (`core-calculus.md`, item 25), computed once here for every
    /// execution path to read.
    pub frames: Vec<Option<frames::Frame>>,
}

impl CheckedModule {
    /// The index of the function named `name`.
    pub fn index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }
}

/// Parses and checks a module, and returns its checked functions when no check reports
/// anything, for the reference interpreter to run.
pub fn check_program(src: &str) -> Result<CheckedModule, Vec<Diagnostic>> {
    program(src, true)
}

/// [`check_program`] without Tier 1: for running programs whose refinements and contracts may
/// fail, as the reference interpreter's own tests do to check its run-time checks.
pub fn check_program_without_proofs(src: &str) -> Result<CheckedModule, Vec<Diagnostic>> {
    program(src, false)
}

fn program(src: &str, prove: bool) -> Result<CheckedModule, Vec<Diagnostic>> {
    let module = parse_module(src).map_err(|d| vec![d])?;
    let b = bodies(src, &module, prove);
    let (functions, body_errors, structs) = (b.functions, b.errors, b.structs);
    combine(verify_module_with(&module, &structs), body_errors)?;
    let frames = functions
        .iter()
        .map(|f| f.as_ref().map(|f| frames::frame(f, &structs)))
        .collect();
    Ok(CheckedModule {
        names: module
            .items
            .iter()
            .filter_map(|i| match i {
                ItemDecl::Fn(f) => Some(f.name.clone()),
                ItemDecl::Unit(_)
                | ItemDecl::Struct(_)
                | ItemDecl::BitStruct(_)
                | ItemDecl::Enum(_)
                | ItemDecl::Trait(_)
                | ItemDecl::Example(_) => None,
            })
            .collect(),
        functions,
        structs,
        frames,
    })
}

/// The canonical form of a module's typed functions and its diagnostics: a header line; for
/// each function whose body checks, its tree as `Tree::render_fn` writes it, then each binding
/// as `  (binding name type)` and each expression as `  (expr id line:col length type)`,
/// types as Mente spells them; then each diagnostic `check_source` reports as `line:col code`.
pub fn dump_typed(src: &str) -> String {
    let mut out = format!("mente-dump {} typed\n", mente_syntax::DUMP_VERSION);
    if let Ok(module) = parse_module(src) {
        for f in bodies(src, &module, true).functions.into_iter().flatten() {
            out.push_str(&mente_syntax::tree::Tree::render_fn(&f.syntax));
            out.push('\n');
            for (b, ty) in f.resolution.bindings.iter().zip(&f.typing.bindings) {
                out.push_str(&format!("  (binding {} {})\n", b.name, ty.spelling()));
            }
            for (i, (e, ty)) in f.syntax.tree.exprs.iter().zip(&f.typing.exprs).enumerate() {
                out.push_str(&format!(
                    "  (expr {} {}:{} {} {})\n",
                    i,
                    e.span.line,
                    e.span.col,
                    e.span.end - e.span.start,
                    ty.spelling()
                ));
            }
        }
    }
    for d in check_source(src).err().unwrap_or_default() {
        out.push_str(&format!(
            "{}:{} {}\n",
            d.primary.line, d.primary.col, d.code
        ));
    }
    out
}

/// Parses each function with a body, then resolves the names and checks the types of each one
/// the body parser reports nothing in, then, when `prove`, runs Tier 1 on each one those report
/// nothing in, function by function in source order. Tier 1 reads each callee's header as it
/// parsed, from the module's functions in order.
/// What checking a module's bodies gives: each function checked when its body parsed cleanly,
/// every diagnostic, Tier 1's report of each function it ran on, and every `todo`.
struct Bodies {
    functions: Vec<Option<CheckedFn>>,
    errors: Vec<Diagnostic>,
    reports: Vec<(String, tier1::Report)>,
    /// For each report, what Tier 1 wrote besides it, as asked.
    extras: Vec<tier1::Extras>,
    todos: Vec<(Span, String)>,
    structs: structs::Structs,
}

fn bodies(src: &str, module: &ModuleFile, prove: bool) -> Bodies {
    bodies_with(src, module, prove, tier1::Ask::default())
}

/// [`bodies`], with Tier 1 writing what `ask` asks for besides each report.
fn bodies_with(src: &str, module: &ModuleFile, prove: bool, ask: tier1::Ask) -> Bodies {
    // The structs first: every function reads their fields and layouts. The module's
    // diagnostics are put in the order of the source where they are combined.
    let parsed_structs = mente_syntax::parse_structs(src, module);
    let parsed_enums = mente_syntax::parse_enums(src, module);
    let mut errors: Vec<Diagnostic> = parsed_structs
        .iter()
        .flat_map(|p| p.errors.iter().cloned())
        .chain(parsed_enums.iter().flat_map(|p| p.errors.iter().cloned()))
        .collect();
    let (structs, declared) = structs::declare(module, &parsed_structs, &parsed_enums);
    errors.extend(declared);
    errors.sort_by_key(|d| d.primary.start);
    // Tier 1 reads the types the declarations give, so it proves no function of a module with a
    // struct or an enum whose declaration `declare` refused, such as one that contains itself
    // (`E0225`), whose field paths go on without end; each body is checked all the same. An
    // error of no type, two functions of one name, leaves Tier 1 to run, as `mn check --watch`
    // runs it on a module of functions alone (`incremental.rs`).
    let types_hold = structs.defs.iter().all(|d| d.ok) && structs.enums.iter().all(|e| e.ok);
    let parsed = parse_qualified(src, module);
    let mut todos: Vec<(Span, String)> = parsed
        .iter()
        .filter_map(|p| p.syntax.as_ref())
        .flat_map(|f| &f.tree.exprs)
        .filter_map(|e| match &e.kind {
            mente_syntax::tree::ExprKind::Todo(reason) => Some((e.span, reason.clone())),
            mente_syntax::tree::ExprKind::Int { .. }
            | mente_syntax::tree::ExprKind::Bool(_)
            | mente_syntax::tree::ExprKind::Name(_)
            | mente_syntax::tree::ExprKind::Unreachable(_)
            | mente_syntax::tree::ExprKind::Unary { .. }
            | mente_syntax::tree::ExprKind::Binary { .. }
            | mente_syntax::tree::ExprKind::Index { .. }
            | mente_syntax::tree::ExprKind::Field { .. }
            | mente_syntax::tree::ExprKind::Call { .. }
            | mente_syntax::tree::ExprKind::Construct { .. }
            | mente_syntax::tree::ExprKind::Variant { .. }
            | mente_syntax::tree::ExprKind::Match { .. }
            | mente_syntax::tree::ExprKind::Array(_)
            | mente_syntax::tree::ExprKind::Repeat { .. }
            | mente_syntax::tree::ExprKind::If { .. }
            | mente_syntax::tree::ExprKind::Error => None,
        })
        .collect();
    todos.sort_by_key(|t| t.0.start);
    let routings: Vec<tier1::Routing> = module
        .items
        .iter()
        .filter_map(|i| match i {
            ItemDecl::Fn(f) => Some(tier1::Routing::of(&f.attrs)),
            ItemDecl::Unit(_)
            | ItemDecl::Struct(_)
            | ItemDecl::BitStruct(_)
            | ItemDecl::Enum(_)
            | ItemDecl::Trait(_)
            | ItemDecl::Example(_) => None,
        })
        .collect();
    let names: Vec<String> = parsed.iter().map(|p| p.name.clone()).collect();
    let signatures: Vec<Option<check::Signature>> = parsed
        .iter()
        .map(|p| p.syntax.as_ref().map(check::Signature::of))
        .collect();
    let headers: Vec<Option<tier1::Header>> = parsed
        .iter()
        .map(|p| p.syntax.as_ref().map(tier1::Header::of))
        .collect();
    let callees: Vec<Option<tier1::Callee>> = headers
        .iter()
        .map(|h| h.as_ref().map(tier1::Callee::of))
        .collect();
    let mut checked = Vec::new();
    let mut reports = Vec::new();
    let mut discharged = Vec::new();
    for (p, routing) in parsed.iter().zip(&routings) {
        errors.extend(p.errors.iter().cloned());
        let done = match (&p.syntax, p.errors.is_empty()) {
            (Some(syntax), true) => {
                let (resolution, mut found) = names::resolve(syntax, &names, &structs);
                let (typing, typed) = check::check(syntax, &resolution, &signatures, &structs, src);
                found.extend(typed);
                found.extend(wide::check(syntax, &resolution, &typing, src));
                let mut obligations = Vec::new();
                let mut variables = 0;
                if prove && types_hold && found.is_empty() && tier1::proves(&names, &p.name) {
                    let f = tier1::Function {
                        syntax,
                        resolution: &resolution,
                        typing: &typing,
                        routing: *routing,
                        structs: &structs,
                    };
                    let (report, written) = tier1::check_with(&f, &callees, src, ask);
                    found.extend(report.diagnostics.iter().cloned());
                    obligations = report.obligations.clone();
                    variables = report.variables;
                    reports.push((p.name.clone(), report));
                    discharged.push(written);
                }
                errors.extend(found);
                Some((resolution, typing, obligations, variables))
            }
            _ => None,
        };
        checked.push(done);
    }
    drop(callees);
    let functions = parsed
        .into_iter()
        .zip(checked)
        .map(|(p, done)| match (p.syntax, done) {
            (Some(syntax), Some((resolution, typing, obligations, variables))) => Some(CheckedFn {
                syntax,
                resolution,
                typing,
                obligations,
                variables,
            }),
            _ => None,
        })
        .collect();
    Bodies {
        functions,
        errors,
        reports,
        extras: discharged,
        todos,
        structs,
    }
}

/// Every function of `module` as [`mente_syntax::parse_bodies`] reads it, each tree qualified by
/// what name resolution finds of the module's enums ([`structs::qualify`]): the trees every check
/// after the parser reads.
pub fn parse_qualified(src: &str, module: &ModuleFile) -> Vec<mente_syntax::ParsedFn> {
    let enums = structs::enum_names(module);
    let mut parsed = mente_syntax::parse_bodies(src, module);
    for p in &mut parsed {
        if let Some(syntax) = &mut p.syntax {
            structs::qualify(&mut syntax.tree, &enums);
        }
    }
    parsed
}

/// Runs full semantic & hardware layout verification on a parsed `.mn` `ModuleFile`, without
/// the sizes of its plain structs, which [`verify_module_with`] takes.
pub fn verify_module(module: &ModuleFile) -> Result<ModuleVerificationReport, Vec<Diagnostic>> {
    verify_module_with(module, &structs::Structs::default())
}

/// [`verify_module`], a `@layout` struct's field of a plain struct sized and aligned by that
/// struct's layout (`core-calculus.md`, item 21), which `structs` holds.
pub fn verify_module_with(
    module: &ModuleFile,
    structs: &structs::Structs,
) -> Result<ModuleVerificationReport, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    let mut layout_proofs = Vec::new();
    let mut bitstruct_proofs = Vec::new();
    let mut total_functions_verified = 0usize;
    let mut branchless_functions_verified = 0usize;
    let mut kernel_functions_verified = 0usize;
    let mut examples_count = 0usize;

    let layout::Tables {
        named_sizes,
        named_aligns,
        bitstruct_bits,
    } = layout::tables(module, structs);

    for item in &module.items {
        match item {
            ItemDecl::Struct(s) => {
                let checked = match layout_attr(s) {
                    Some(Ok(checked)) => Some(checked),
                    Some(Err(e)) => {
                        errors.push(e);
                        None
                    }
                    None => None,
                };
                if let Some((size, align, _)) = checked {
                    // An enum has a layout (`core-calculus.md`, item 21), but a field of a
                    // `@layout` struct that holds one is a construct no milestone brings yet.
                    if let Some(f) = s
                        .fields
                        .iter()
                        .find(|f| structs.is_enum(element_type(&f.ty)))
                    {
                        errors.push(Diagnostic::error(
                            "E0109",
                            f.ty_span,
                            mente_syntax::diagnostics::later_message(
                                "an enum as a field of a `@layout` struct",
                                None,
                            ),
                        ));
                        continue;
                    }
                    match verify_explicit_struct_layout(s, size, align, &named_sizes, &named_aligns)
                    {
                        Ok(proof) => layout_proofs.push(proof),
                        Err(e) => errors.push(e),
                    }
                }
            }
            ItemDecl::BitStruct(bs) => match verify_bitstruct_layout(bs, &bitstruct_bits) {
                Ok(proof) => bitstruct_proofs.push(proof),
                Err(e) => errors.push(e),
            },
            ItemDecl::Fn(f) => {
                if let Err(e) = verify_function_contracts(f) {
                    errors.push(e);
                } else {
                    if f.attrs.iter().any(|a| a.name == "total") || !f.decreases.is_empty() {
                        total_functions_verified += 1;
                    }
                    if f.attrs.iter().any(|a| a.name == "branchless") {
                        branchless_functions_verified += 1;
                    }
                    if f.attrs.iter().any(|a| a.name == "kernel") {
                        kernel_functions_verified += 1;
                    }
                }
            }
            ItemDecl::Example(_) => {
                examples_count += 1;
            }
            ItemDecl::Unit(_) | ItemDecl::Enum(_) | ItemDecl::Trait(_) => {}
        }
    }

    if errors.is_empty() {
        Ok(ModuleVerificationReport {
            module_path: module.module_path.clone(),
            layout_proofs,
            bitstruct_proofs,
            total_functions_verified,
            branchless_functions_verified,
            kernel_functions_verified,
            examples_count,
        })
    } else {
        Err(errors)
    }
}

/// Verifies `@total` termination obligations and `@branchless` / `@no_alloc` syntactic rules on a function.
pub fn verify_function_contracts(f: &FnDecl) -> Result<(), Diagnostic> {
    let is_total = f.attrs.iter().any(|a| a.name == "total");
    let is_branchless = f.attrs.iter().any(|a| a.name == "branchless");

    // 0. A returned view may borrow only from a parameter the caller keeps: never an `own` one.
    for (i, source) in f.provenance.iter().enumerate() {
        let at = f.provenance_spans.get(i).copied().unwrap_or(f.name_span);
        match f.params.iter().find(|p| &p.name == source) {
            None => {
                let d = Diagnostic::error(
                    "E0311",
                    at,
                    format!(
                        "`from {}` in the signature of `{}` names no parameter of `{}`",
                        source, f.name, f.name
                    ),
                );
                // With one parameter a view can borrow from, that is the one meant.
                let kept: Vec<_> = f
                    .params
                    .iter()
                    .filter(|p| p.mode != ParamMode::Own)
                    .collect();
                return Err(match kept.as_slice() {
                    [only] => d.with_fix(
                        Applicability::Suggestion,
                        format!("borrow from `{}`", only.name),
                        at,
                        only.name.clone(),
                    ),
                    _ => d,
                });
            }
            Some(p) if p.mode == ParamMode::Own => {
                return Err(Diagnostic::error(
                    "E0311",
                    at,
                    format!(
                        "`from {}` in the signature of `{}` names an `own` parameter; a returned view cannot borrow from a value the callee consumes",
                        source, f.name
                    ),
                ));
            }
            Some(_) => {}
        }
    }

    // 1. A `@total` function that loops with `while` gives a `decreases` measure.
    if is_total {
        let has_while = f.body_tokens.contains(&TokenKind::While);
        let has_decreases =
            !f.decreases.is_empty() || f.body_tokens.contains(&TokenKind::Decreases);
        if has_while && !has_decreases {
            return Err(Diagnostic::error(
                "E0501",
                f.name_span,
                format!(
                    "function `{}` is marked `@total` and loops with `while`, but gives no `decreases` measure",
                    f.name
                ),
            ));
        }
    }

    // 2. A `@branchless` body has no `while` loop and no `raise`.
    if is_branchless {
        if f.body_tokens.contains(&TokenKind::While) {
            return Err(Diagnostic::error(
                "E0401",
                f.name_span,
                format!(
                    "function `{}` is marked `@branchless`, but contains a `while` loop",
                    f.name
                ),
            ));
        }
        if f.body_tokens.contains(&TokenKind::Raise) {
            return Err(Diagnostic::error(
                "E0402",
                f.name_span,
                format!(
                    "function `{}` is marked `@branchless`, but contains `raise`",
                    f.name
                ),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_typed_functions_dump_with_their_diagnostics() {
        assert_eq!(
            dump_typed("module m\npub fn f(x: u8) -> u8 {\n  val y = x + 1\n  y\n}\npub fn g() -> u8 = zz\n"),
            "mente-dump 3 typed\n(fn f ((x u8)) u8 () {(val y (+ x 1)); y})\n  (binding x u8)\n  (binding y u8)\n  (expr 0 3:11 1 u8)\n  (expr 1 3:15 1 integer)\n  (expr 2 3:11 5 u8)\n  (expr 3 4:3 1 u8)\n(fn g () u8 () (= zz))\n  (expr 0 6:20 2 _)\n6:20 E0201\n"
        );
        assert_eq!(
            dump_typed("module m\nlet x = 1\n"),
            "mente-dump 3 typed\n2:1 E0111\n"
        );
    }

    #[test]
    fn test_listings_name_every_todo_and_trusted_clause() {
        let src = "module m\n@prove\npub fn h(x: u8) -> u8\n  requires x != 3\n= todo(\"b\")\npub fn f(x: u8) -> u8 = if x > 2 { todo(\"a\") } else { todo(\"c\") }\npub fn g() -> u8 = zz + todo(\"d\")\n";
        let l = listings(src);
        let todos: Vec<(u32, u32, &str)> = l
            .todos
            .iter()
            .map(|(s, r)| (s.line, s.col, r.as_str()))
            .collect();
        assert_eq!(
            todos,
            [(5, 3, "b"), (6, 36, "a"), (6, 55, "c"), (7, 25, "d")]
        );
        let axioms: Vec<(u32, &str)> = l.axioms.iter().map(|(s, t)| (s.line, t.as_str())).collect();
        assert_eq!(axioms, [(4, "requires x != 3 of h")]);
        let two = "module m\n@prove\npub fn b(x: u8)\n  requires x != 1\n{\n}\n@prove\npub fn a(x: u8)\n  requires x != 2\n{\n}\n";
        let lines: Vec<u32> = listings(two).axioms.iter().map(|t| t.0.line).collect();
        assert_eq!(lines, [4, 9]);
        assert_eq!(listings("module m\nlet x = 1\n"), Listings::default());
        // Tier 1 reports on each function whose names and types check.
        let names: Vec<String> = tier1_reports(src).into_iter().map(|r| r.0).collect();
        assert_eq!(names, ["h", "f"]);
        assert!(tier1_reports("module m\nlet x = 1\n").is_empty());
    }

    #[test]
    fn test_check_module_reports_declarations_first_then_bodies() {
        let clean = "module m\npub fn f(x: i32) -> i32 = x + 1\n";
        assert!(check_source(clean).is_ok());
        let body_only = "module m\npub fn f(x: i32) -> i32 = x +\n";
        let codes = |src: &str| -> Vec<&str> {
            check_source(src)
                .err()
                .unwrap_or_default()
                .iter()
                .map(|d| d.code)
                .collect()
        };
        assert_eq!(codes(body_only), vec!["E0110"]);
        let both = "module m\n@layout(size: 16, align: 8)\npub struct Hole {\n  @bytes(0..8) pub id: u64\n  @bytes(12..16) pub rest: u32\n}\npub fn f(x: i32) -> i32 = x +\n";
        assert_eq!(codes(both), vec!["E0413", "E0110"]);
    }

    #[test]
    fn test_rejects_struct_with_padding_hole() {
        let bad_code = r#"
module test.bad
@layout(size: 64, align: 64)
pub struct BrokenCell {
  @bytes(0..8)   pub id: u64
  @bytes(12..64) pub rest: [52]u8
}
"#;
        let m = parse_module(bad_code).unwrap();
        let res = verify_module(&m);
        assert!(res.is_err());
        let errs = res.unwrap_err();
        assert!(errs[0].to_string().contains("E0413"));
    }

    #[test]
    fn test_layout_field_sized_by_module_bitstruct() {
        let code = r#"
module test.sized
pub bitstruct Tag : u16 {
  @bits(0..16) pub raw: u16
}
@layout(size: 8, align: 8)
pub struct Packet {
  @bytes(0..2) pub tag: Tag
  @bytes(2..8) pub rest: [6]u8
}
"#;
        let rep = verify_module(&parse_module(code).unwrap()).unwrap();
        assert_eq!(rep.layout_proofs.len(), 1);

        let too_wide = code.replace("@bytes(0..2) pub tag", "@bytes(0..4) pub tag");
        let errs = verify_module(&parse_module(&too_wide).unwrap()).unwrap_err();
        assert!(errs[0].to_string().contains("E0415"));
    }

    #[test]
    fn test_bitstruct_field_type_must_match_its_range() {
        let code = r#"
module test.bits
pub bitstruct Flags : u8 {
  @bits(0..3) pub mode: u3
  @bits(3..4) pub armed: bool
  @bits(4..8) pub level: i4
}
@layout(size: 4, align: 1)
pub struct Frame {
  @bytes(0..3) pub seq: u24
  @bytes(3..4) pub flags: Flags
}
"#;
        let rep = verify_module(&parse_module(code).unwrap()).unwrap();
        assert_eq!(
            (rep.bitstruct_proofs.len(), rep.layout_proofs.len()),
            (1, 1)
        );

        let narrow = code.replace("pub mode: u3", "pub mode: u2");
        let errs = verify_module(&parse_module(&narrow).unwrap()).unwrap_err();
        assert!(errs[0].to_string().contains("E0424"), "{:?}", errs);

        let opaque = code.replace("pub level: i4", "pub level: Level");
        let errs = verify_module(&parse_module(&opaque).unwrap()).unwrap_err();
        assert!(errs[0].to_string().contains("E0425"), "{:?}", errs);
    }

    #[test]
    fn test_layout_fields_sit_at_aligned_offsets() {
        let code = r#"
module test.aligned
@layout(size: 16, align: 8)
pub struct Record {
  @bytes(0..4)  pub tag: u32
  @bytes(4..12) pub value: u64
  @bytes(12..16) pub crc: u32
}
"#;
        let errs = verify_module(&parse_module(code).unwrap()).unwrap_err();
        assert!(
            errs[0].to_string().contains("E0418") && errs[0].to_string().contains("value"),
            "{:?}",
            errs
        );

        let packed = code.replace(
            "@layout(size: 16, align: 8)",
            "@packed\n@layout(size: 16, align: 8)",
        );
        assert!(verify_module(&parse_module(&packed).unwrap()).is_ok());

        let under_aligned = r#"
module test.under
@layout(size: 8, align: 4)
pub struct Wide {
  @bytes(0..8) pub value: u64
}
"#;
        let errs = verify_module(&parse_module(under_aligned).unwrap()).unwrap_err();
        assert!(errs[0].to_string().contains("E0419"), "{:?}", errs);
    }

    #[test]
    fn test_rejects_layout_field_of_unknown_size() {
        let code = r#"
module test.unknown
@layout(size: 16, align: 8)
pub struct Holder {
  @bytes(0..8)  pub len: usize
  @bytes(8..16) pub mystery: Opaque
}
"#;
        let errs = verify_module(&parse_module(code).unwrap()).unwrap_err();
        assert!(errs[0].to_string().contains("E0417") && errs[0].to_string().contains("usize"));
    }

    #[test]
    fn test_from_clause_must_name_a_kept_parameter() {
        let ok = r#"
module test.views
pub fn head(data: []u8, n: usize) -> []u8 from data
"#;
        assert!(verify_module(&parse_module(ok).unwrap()).is_ok());

        let unknown = ok.replace("from data", "from buffer");
        let errs = verify_module(&parse_module(&unknown).unwrap()).unwrap_err();
        assert!(
            errs[0].to_string().contains("E0311")
                && errs[0].to_string().contains("names no parameter")
        );

        let consumed = ok.replace("head(data:", "head(own data:");
        let errs = verify_module(&parse_module(&consumed).unwrap()).unwrap_err();
        assert!(
            errs[0].to_string().contains("E0311")
                && errs[0].to_string().contains("`own` parameter")
        );
    }

    #[test]
    fn test_rejects_branchless_with_while_loop() {
        let bad_branchless = r#"
module test.bad_branchless
@branchless
pub fn bad_loop(x: i32) -> i32 {
  var i = x
  while i > 0 {
    i -= 1
  }
  i
}
"#;
        let m = parse_module(bad_branchless).unwrap();
        let res = verify_module(&m);
        assert!(res.is_err());
        assert!(res.unwrap_err()[0].to_string().contains("E0401"));
    }

    #[test]
    fn test_rejects_branchless_with_raise() {
        let code = r#"
module test.jumps
@branchless
pub fn pick(x: i32) -> i32 ! RangeError {
  raise RangeError.TooBig
}
"#;
        let errs = verify_module(&parse_module(code).unwrap()).unwrap_err();
        assert!(errs[0].to_string().contains("E0402"), "{:?}", errs);
        // The same body is accepted without the contract: raise is ordinary control flow.
        let plain = code.replace("@branchless\n", "");
        assert!(verify_module(&parse_module(&plain).unwrap()).is_ok());
    }

    /// Tier 1 proves no function of a module with a struct or an enum whose declaration fails:
    /// a struct that contains itself, used as a parameter, once sent Tier 1 down its field paths
    /// without end (the nightly fuzzer, 2026-11-14 to 11-20), and a function beside it whose
    /// refinement Tier 1 cannot prove reports nothing of it until the types check. Its body is
    /// still checked. An error of no type, two functions of one name, leaves Tier 1 to run, as
    /// `mn check --watch` runs it on a module of functions alone: the fuzzer found the two
    /// disagreeing when every declaration's error stopped Tier 1.
    #[test]
    fn test_tier1_proves_no_function_of_a_module_whose_types_fail() {
        let codes = |src: &str| -> Vec<&'static str> {
            check_source(src)
                .err()
                .unwrap_or_default()
                .iter()
                .map(|d| d.code)
                .collect()
        };
        let cycle = "module m\nstruct P {\n  a: P\n}\nfn e(p: P) -> u8 = 0\n";
        assert_eq!(codes(cycle), ["E0225"]);
        let beside = "module m\nstruct P {\n  a: P\n}\npub fn f(x: u8) -> u8[0..10] = x\n";
        assert_eq!(codes(beside), ["E0225"]);
        let held = beside.replace("  a: P\n", "  a: u8\n");
        assert_eq!(codes(&held), ["E0502"]);
        // A body's own error is reported beside the declarations'.
        let body = beside.replace("= x\n", "= y\n");
        assert_eq!(codes(&body), ["E0225", "E0201"]);
        let enums = "module m\nenum E {\n  V(e: E)\n}\npub fn f(x: u8) -> u8[0..10] = x\n";
        assert_eq!(codes(enums), ["E0225"]);
        // Two functions of one name: neither is proven (`tier1::proves`), in a session as in a
        // whole check, so the narrowing Tier 1 would prove always traps draws no `E0508`.
        for twice in [
            "module m\nfn f()\nfn f() -> u8 = 0xFFF\n",
            "module m\npub fn f() -> u8 = 2\npub fn f() -> u8 = -0xFFFFFFFFFFFFFFFF_u128\n",
        ] {
            assert_eq!(codes(twice), ["E0228"], "{}", twice);
            let session = incremental::Session::new(twice);
            assert_eq!(
                session.diagnostics(),
                check_source(twice).err().unwrap_or_default().as_slice()
            );
        }
    }

    #[test]
    fn test_total_while_without_decreases_is_rejected() {
        let code = r#"
module test.total
@total
pub fn countdown(n: u32) -> u32 {
  var i = n
  while i > 0 {
    i -= 1
  }
  i
}
"#;
        let errs = verify_module(&parse_module(code).unwrap()).unwrap_err();
        assert!(
            errs[0].to_string().contains("E0501") && errs[0].to_string().contains("countdown"),
            "{:?}",
            errs
        );

        // A measure in the signature or in the body satisfies the token check.
        let in_signature = code.replace("-> u32 {", "-> u32\n  decreases n\n{");
        let rep = verify_module(&parse_module(&in_signature).unwrap()).unwrap();
        assert_eq!(rep.total_functions_verified, 1);
        let in_body = code.replace("while i > 0 {", "while i > 0 {\n    decreases i");
        assert!(verify_module(&parse_module(&in_body).unwrap()).is_ok());

        // Without @total, a loop needs no measure; without a loop, @total needs none either.
        let plain = code.replace("@total\n", "");
        assert!(verify_module(&parse_module(&plain).unwrap()).is_ok());
        let no_loop = "module test.total\n@total\npub fn id(n: u32) -> u32 {\n  n\n}\n";
        assert!(verify_module(&parse_module(no_loop).unwrap()).is_ok());
    }

    #[test]
    fn test_fixed_point_fields_have_their_byte_sizes() {
        let code = r#"
module test.fixed
@layout(size: 8, align: 4)
pub struct Sample {
  @bytes(0..4) pub level: q16.16
  @bytes(4..6) pub gain: q1.15
  @bytes(6..7) pub weight: q0.8
  @bytes(7..8) pub pad: u8
}
"#;
        let rep = verify_module(&parse_module(code).unwrap()).unwrap();
        assert_eq!(rep.layout_proofs[0].total_bytes, LayoutNum::small(8));

        let wrong = code
            .replace(
                "@bytes(0..4) pub level: q16.16",
                "@bytes(0..2) pub level: q16.16",
            )
            .replace("@bytes(4..6) pub gain", "@bytes(2..6) pub gain");
        let errs = verify_module(&parse_module(&wrong).unwrap()).unwrap_err();
        assert!(
            errs[0].to_string().contains("E0415")
                && errs[0].to_string().contains("requires 4 bytes"),
            "{:?}",
            errs
        );
    }

    fn first_error(code: &str) -> Diagnostic {
        let errs = verify_module(&parse_module(code).unwrap()).unwrap_err();
        errs[0].clone()
    }

    #[test]
    fn test_layout_rejects_bad_geometry() {
        let good = "module test.geo\n@layout(size: 8, align: 4)\npub struct Pair {\n  @bytes(0..4) pub a: u32\n  @bytes(4..8) pub b: u32\n}\n";
        let rep = verify_module(&parse_module(good).unwrap()).unwrap();
        assert_eq!(
            (
                rep.layout_proofs[0].total_bytes,
                rep.layout_proofs[0].align_bytes
            ),
            (LayoutNum::small(8), LayoutNum::small(4))
        );
        assert!(rep.layout_proofs[0]
            .markdown_table
            .contains("| `[4..8)` | `4 B` | `b` | `u32` |"));

        let cases = [
            (good.replace("align: 4", "align: 3"), "E0410"),
            (
                good.replace("size: 8, align: 4", "size: 6, align: 4"),
                "E0411",
            ),
            (good.replace("@bytes(4..8) pub b", "pub b"), "E0412"),
            (
                good.replace("@bytes(0..4) pub a: u32", "@bytes(0..0) pub a: u32")
                    .replace("@bytes(4..8)", "@bytes(0..4)"),
                "E0414",
            ),
            (good.replace("size: 8", "size: 12"), "E0416"),
            // Whose fix would lie past the bytes a value may hold, as the fuzzer's findings 16
            // and 17 lay past `usize`.
            (good.replace("align: 4", "align: 4294967295"), "E0410"),
            (good.replace("size: 8", "size: 4294967295"), "E0411"),
        ];
        for (code, expected) in cases {
            let err = first_error(&code);
            assert!(
                err.to_string().contains(expected),
                "expected {}: {}",
                expected,
                err
            );
        }
    }

    #[test]
    fn test_report_counts_contract_functions() {
        let code = r#"
module test.counts
@total
pub fn a(n: u32) -> u32 = n
pub fn b(n: u32) -> u32
  decreases n
= n
@branchless
pub fn c(n: u32) -> u32 = n
@kernel
pub fn d(n: u32) -> u32 = n
@inline
pub fn e(n: u32) -> u32 = n
@example("one")
{
  a(1)
}
"#;
        let rep = verify_module(&parse_module(code).unwrap()).unwrap();
        assert_eq!(
            (
                rep.total_functions_verified,
                rep.branchless_functions_verified,
                rep.kernel_functions_verified,
                rep.examples_count
            ),
            (2, 1, 1, 1)
        );
        assert_eq!(rep.module_path, "test.counts");
    }

    #[test]
    fn test_layout_field_may_be_a_layout_struct() {
        let code = r#"
module test.nested
@layout(size: 8, align: 8)
pub struct Stamp {
  @bytes(0..8) pub nanos: u64
}
@layout(size: 16, align: 8)
pub struct Event {
  @bytes(0..8) pub at: Stamp
  @bytes(8..16) pub value: f64
}
"#;
        let rep = verify_module(&parse_module(code).unwrap()).unwrap();
        assert_eq!(rep.layout_proofs.len(), 2);
        // Stamp's declared alignment applies where it is used.
        let misplaced = code
            .replace("@layout(size: 16, align: 8)\npub struct Event", "@packed\n@layout(size: 20, align: 4)\npub struct Event")
            .replace("@bytes(0..8) pub at: Stamp\n  @bytes(8..16) pub value: f64", "@bytes(0..4) pub tag: u32\n  @bytes(4..12) pub at: Stamp\n  @bytes(12..20) pub value: f64");
        assert!(verify_module(&parse_module(&misplaced).unwrap()).is_ok());
        let unpacked = misplaced.replace("@packed\n", "");
        assert!(first_error(&unpacked).to_string().contains("E0418"));
    }

    /// A module's diagnostics come in the order of its source, by the start of their primary
    /// spans, whichever check found them: a declaration's after a function before it, a
    /// struct's after a function, and a function's type error before its unknown name. The
    /// incremental session lists them alike.
    #[test]
    fn test_diagnostics_are_listed_in_source_order() {
        let codes = |src: &str| {
            check_source(src)
                .err()
                .unwrap_or_default()
                .iter()
                .map(|d| d.code)
                .collect::<Vec<_>>()
        };
        let layout = "module m\npub fn f() -> u8 = 300_u8\n@layout(size: 4, align: 4)\npub struct R {\n  @bytes(0..2) pub a: u16\n}\n";
        assert_eq!(codes(layout), ["E0221", "E0416"]);
        let plain = "module m\npub fn f() -> u8 = 300_u8\npub struct S {\n  a: u8,\n}\n";
        assert_eq!(codes(plain), ["E0221", "E0128"]);
        let body = "module m\npub fn g(x: u8) -> u8 {\n  val a: bool = 1\n  _ = a\n  nope + x\n}\npub fn h() -> u8 = 300_u8\n";
        assert_eq!(codes(body), ["E0203", "E0201", "E0221"]);
        for src in [layout, plain, body] {
            let whole = check_source(src).err().unwrap_or_default();
            let starts: Vec<usize> = whole.iter().map(|d| d.primary.start).collect();
            assert!(starts.windows(2).all(|w| w[0] <= w[1]), "{:?}", starts);
            assert_eq!(incremental::Session::new(src).diagnostics(), &whole[..]);
        }
    }

    /// A `@layout` struct's field of a plain struct is sized and aligned by the plain struct's
    /// own layout (`core-calculus.md`, item 21): `Pair` takes 8 bytes, aligned to 4.
    #[test]
    fn test_layout_field_may_be_a_plain_struct() {
        let code = "module m
pub struct Pair {
  a: u32
  b: u8
}
@layout(size: 16, align: 8)
pub struct Rec {
  @bytes(0..8) pub id: u64
  @bytes(8..16) pub p: Pair
}
";
        assert!(check_source(code).is_ok(), "{:?}", check_source(code).err());
        let codes = |src: &str| -> Vec<&str> {
            check_source(src)
                .err()
                .unwrap_or_default()
                .iter()
                .map(|d| d.code)
                .collect()
        };
        let narrow = code.replace(
            "@bytes(8..16) pub p: Pair",
            "@bytes(8..13) pub p: Pair
  @bytes(13..16) pub pad: [3]u8",
        );
        assert_eq!(codes(&narrow), ["E0415"]);
        let misaligned = code
            .replace("@layout(size: 16, align: 8)", "@layout(size: 12, align: 4)")
            .replace(
                "@bytes(0..8) pub id: u64
  @bytes(8..16) pub p: Pair",
                "@bytes(0..2) pub id: u16
  @bytes(2..10) pub p: Pair
  @bytes(10..12) pub pad: u16",
            );
        assert_eq!(codes(&misaligned), ["E0418"]);
        // Without the plain structs' layouts, as `verify_module` alone reads a module, the
        // size is unknown.
        assert!(first_error(code).to_string().contains("E0417"));
    }

    /// An enum has a layout (`core-calculus.md`, item 21), but a field of a `@layout` struct
    /// that holds one, directly or in an array, is a construct no milestone brings yet
    /// (`E0109`), not a field whose size is unknown (`E0417`).
    #[test]
    fn test_an_enum_field_of_a_layout_struct_is_a_later_construct() {
        for (layout, field) in [
            ("@layout(size: 1, align: 1)", "@bytes(0..1) pub e: E"),
            ("@layout(size: 2, align: 1)", "@bytes(0..2) pub es: [2]E"),
        ] {
            let src = format!(
                "module m\npub enum E {{\n  A\n  B\n}}\n{}\npub struct Stamp {{\n  {}\n}}\n",
                layout, field
            );
            let d = check_source(&src).unwrap_err();
            assert_eq!(
                d.iter().map(|d| d.code).collect::<Vec<_>>(),
                ["E0109"],
                "{}",
                src
            );
            assert_eq!(
                d[0].message,
                "an enum as a field of a `@layout` struct is in no milestone yet; M2 does not have it"
            );
        }
    }

    #[test]
    fn test_field_sizes_and_alignments_follow_their_types() {
        // u12 is not a whole number of bytes, so it belongs in a bitstruct.
        let odd = "module test.sizes\n@layout(size: 2, align: 2)\npub struct Odd {\n  @bytes(0..2) pub v: u12\n}\n";
        assert!(first_error(odd).to_string().contains("E0417"));
        // An array is aligned like its element: [2]u32 cannot start at byte 2.
        let arr = "module test.sizes\n@layout(size: 12, align: 4)\npub struct Arr {\n  @bytes(0..2) pub tag: u16\n  @bytes(2..10) pub vals: [2]u32\n  @bytes(10..12) pub crc: u16\n}\n";
        let err = first_error(arr);
        assert!(
            err.to_string().contains("E0418") && err.to_string().contains("vals"),
            "{}",
            err
        );
        // Leading zeros do not spell a width.
        let zero = "module test.sizes\npub bitstruct Z : u8 {\n  @bits(0..7) pub v: u07\n  @bits(7..8) pub f: bool\n}\n";
        assert!(first_error(zero).to_string().contains("E0425"));
        // Every backing width from u8 to u128 is accepted.
        for (backing, bits) in [
            ("u8", 8),
            ("u16", 16),
            ("u32", 32),
            ("u64", 64),
            ("u128", 128),
        ] {
            let code = format!(
                "module test.sizes\npub bitstruct W : {} {{\n  @bits(0..{}) pub raw: u{}\n}}\n",
                backing, bits, bits
            );
            let rep = verify_module(&parse_module(&code).unwrap()).unwrap();
            assert_eq!(rep.bitstruct_proofs[0].total_bits, LayoutNum::small(bits));
        }
    }

    #[test]
    fn test_bitstruct_rejects_gaps_and_shortfalls() {
        let good = "module test.bits\npub bitstruct Flags : u8 {\n  @bits(0..4) pub lo: u4\n  @bits(4..8) pub hi: u4\n}\n";
        let rep = verify_module(&parse_module(good).unwrap()).unwrap();
        assert_eq!(
            (
                rep.bitstruct_proofs[0].name.as_str(),
                rep.bitstruct_proofs[0].total_bits
            ),
            ("Flags", LayoutNum::small(8))
        );

        let cases = [
            (good.replace(": u8 {", ": u12 {"), "E0420"),
            (
                good.replace("@bits(4..8) pub hi: u4", "@bits(5..8) pub hi: u3"),
                "E0421",
            ),
            (
                good.replace("@bits(0..4) pub lo: u4", "@bits(0..0) pub lo: u4")
                    .replace("@bits(4..8)", "@bits(0..4)"),
                "E0422",
            ),
            (
                good.replace("@bits(4..8) pub hi: u4", "@bits(4..7) pub hi: u3"),
                "E0423",
            ),
        ];
        for (code, expected) in cases {
            let err = first_error(&code);
            assert!(
                err.to_string().contains(expected),
                "expected {}: {}",
                expected,
                err
            );
        }
    }

    /// The first diagnostic of a module that must fail, and the text its primary span covers.
    fn diagnosed(src: &str) -> (Diagnostic, String) {
        let d = check_source(src).unwrap_err().remove(0);
        let text = src[d.primary.start..d.primary.end].to_string();
        (d, text)
    }

    fn layout(attr: &str, fields: &str) -> String {
        format!("module t\n{}\npub struct S {{\n{}}}\n", attr, fields)
    }

    #[test]
    fn test_diagnostics_point_at_their_source() {
        for (src, code, text) in [
            (layout("@layout(size: 8, align: 3)", "  @bytes(0..8) pub a: u64\n"), "E0410", "@layout(size: 8, align: 3)"),
            (layout("@layout(size: 8, align: 4)", "  @bytes(0..4) pub a: u32\n  pub b: u32\n"), "E0412", "b"),
            (layout("@layout(size: 8, align: 4)", "  @bytes(0..4) pub a: u32\n  @bytes(6..8) pub b: u16\n"), "E0413", "@bytes(6..8)"),
            (layout("@layout(size: 8, align: 8)", "  @bytes(0..8) pub n: usize\n"), "E0417", "usize"),
            (layout("@layout(size: 8, align: 4)", "  @bytes(0..8) pub v: u64\n"), "E0419", "u64"),
            (String::from("module t\npub bitstruct B : u12 {\n  @bits(0..12) pub v: u12\n}\n"), "E0420", "u12"),
            (String::from("module t\npub bitstruct B : u8 {\n  @bits(0..3) pub m: u2\n  @bits(3..8) pub r: u5\n}\n"), "E0424", "u2"),
            (String::from("module t\npub fn head(data: []u8) -> []u8 from buffer\n"), "E0311", "buffer"),
            (String::from("module t\n@total\npub fn countdown(n: u32) -> u32 {\n  while n > 0 {\n  }\n  n\n}\n"), "E0501", "countdown"),
        ] {
            let (d, at) = diagnosed(&src);
            assert_eq!((d.code, at.as_str()), (code, text), "{}", d);
        }
    }

    #[test]
    fn test_machine_applicable_fix_gives_the_only_valid_range() {
        // A field without @bytes: contiguity and its type fix the range, so the fix is certain.
        let src = "module t\n@layout(size: 8, align: 4)\npub struct S {\n  @bytes(0..4) pub a: u32\n  pub b: u32\n}\n";
        let (d, _) = diagnosed(src);
        assert_eq!(d.code, "E0412");
        assert_eq!(d.fixes[0].applicability, Applicability::MachineApplicable);
        let fixed = mente_syntax::diagnostics::apply_fix(src, &d.fixes[0]).unwrap();
        assert!(fixed.contains("  @bytes(4..8) pub b: u32\n"), "{}", fixed);
        assert!(check_source(&fixed).is_ok(), "{}", fixed);
        // When the size of the type is unknown, no range is certain and no fix is offered.
        let (d, _) = diagnosed(&src.replace("pub b: u32", "pub b: Opaque"));
        assert_eq!((d.code, d.fixes.len()), ("E0412", 0));
    }

    #[test]
    fn test_suggested_fixes_repair_the_module() {
        for (src, code) in [
            (layout("@layout(size: 8, align: 3)", "  @bytes(0..4) pub a: u32\n  @bytes(4..8) pub b: u32\n"), "E0410"),
            (layout("@layout(size: 6, align: 4)", "  @bytes(0..4) pub a: u32\n  @bytes(4..8) pub b: u32\n"), "E0411"),
            (layout("@layout(size: 8, align: 4)", "  @bytes(0..4) pub a: u32\n  @bytes(6..10) pub b: u32\n"), "E0413"),
            (layout("@layout(size: 8, align: 4)", "  @bytes(0..4) pub a: u32\n  @bytes(4..4) pub b: u32\n"), "E0414"),
            (layout("@layout(size: 8, align: 4)", "  @bytes(0..4) pub a: u32\n  @bytes(4..6) pub b: u32\n"), "E0415"),
            (layout("@layout(size: 8, align: 4)", "  @bytes(0..4) pub a: u32\n"), "E0416"),
            (layout("@layout(size: 8, align: 4)", "  @bytes(0..2) pub a: u16\n  @bytes(2..6) pub b: u32\n  @bytes(6..8) pub c: u16\n"), "E0418"),
            (layout("@layout(size: 8, align: 4)", "  @bytes(0..8) pub v: u64\n"), "E0419"),
            (String::from("module t\npub bitstruct B : u8 {\n  @bits(0..4) pub a: u4\n  @bits(5..9) pub b: u4\n}\n"), "E0421"),
            (String::from("module t\npub bitstruct B : u16 {\n  @bits(0..8) pub a: u8\n}\n"), "E0423"),
            (String::from("module t\npub bitstruct B : u8 {\n  @bits(0..3) pub m: u2\n  @bits(3..8) pub r: u5\n}\n"), "E0424"),
            (String::from("module t\npub bitstruct B : u8 {\n  @bits(0..1) pub on: u2\n  @bits(1..8) pub r: u7\n}\n"), "E0424"),
            (String::from("module t\npub fn head(data: []u8, own key: []u8) -> []u8 from buffer\n"), "E0311"),
        ] {
            let (d, _) = diagnosed(&src);
            assert_eq!(d.code, code, "{}", d);
            assert_eq!(d.fixes[0].applicability, Applicability::Suggestion, "{}", d);
            let fixed = mente_syntax::diagnostics::apply_fix(&src, &d.fixes[0]).unwrap();
            assert!(
                check_source(&fixed).is_ok(),
                "{} did not repair:\n{}\n{:?}",
                code,
                fixed,
                check_source(&fixed).err()
            );
        }
        // E0420 suggests the next backing width; the fields still decide whether it is enough.
        let (d, _) = diagnosed("module t\npub bitstruct B : u12 {\n  @bits(0..12) pub v: u12\n}\n");
        assert_eq!(
            (d.code, d.fixes[0].edits[0].replacement.as_str()),
            ("E0420", "u16")
        );
    }

    #[test]
    fn test_gap_and_overlap_are_told_apart() {
        let (d, _) = diagnosed(&layout(
            "@layout(size: 8, align: 4)",
            "  @bytes(0..4) pub a: u32\n  @bytes(6..8) pub b: u16\n",
        ));
        assert!(d.message.contains("has a gap before field `b`: it starts at byte 6, but the previous field ends at byte 4"), "{}", d);
        let (d, _) = diagnosed(&layout(
            "@layout(size: 8, align: 4)",
            "  @bytes(0..4) pub a: u32\n  @bytes(2..6) pub b: u32\n",
        ));
        assert!(
            d.message.contains(
                "field `b` overlaps the previous field: it starts at byte 2, before byte 4"
            ),
            "{}",
            d
        );
        let bits = |b: &str| {
            format!("module t\npub bitstruct B : u8 {{\n  @bits(0..4) pub a: u4\n  @bits({}) pub b: u4\n}}\n", b)
        };
        let (d, _) = diagnosed(&bits("5..9"));
        assert!(
            d.message
                .contains("has a gap before field `b`: it starts at bit 5"),
            "{}",
            d
        );
        let (d, _) = diagnosed(&bits("3..7"));
        assert!(
            d.message
                .contains("has an overlap before field `b`: it starts at bit 3"),
            "{}",
            d
        );
    }

    #[test]
    fn test_fixes_are_offered_only_when_they_fit() {
        // E0416: no fields, or fields ending off the alignment, leave no size to suggest.
        let (d, _) = diagnosed(&layout("@layout(size: 8, align: 4)", ""));
        assert_eq!((d.code, d.fixes.len()), ("E0416", 0));
        let (d, _) = diagnosed(&layout(
            "@layout(size: 8, align: 4)",
            "  @bytes(0..4) pub a: u32\n  @bytes(4..6) pub b: u16\n",
        ));
        assert_eq!((d.code, d.fixes.len()), ("E0416", 0));

        // E0424: an integer field takes the width of its range, keeping its sign; one bit is a
        // bool; a nested bitstruct cannot change width.
        let field = |ty: &str, range: &str| {
            format!("module t\npub bitstruct In : u8 {{\n  @bits(0..8) pub r: u8\n}}\npub bitstruct B : u16 {{\n  @bits({}) pub f: {}\n  @bits(8..16) pub rest: u8\n}}\n", range, ty)
        };
        for (ty, range, fix) in [
            ("u2", "0..8", Some("u8")),
            ("i2", "0..8", Some("i8")),
            ("bool", "0..8", Some("u8")),
            ("u4", "0..1", None),
            ("In", "0..4", None),
        ] {
            let src = if ty == "u4" {
                String::from("module t\npub bitstruct B : u8 {\n  @bits(0..1) pub on: u4\n  @bits(1..8) pub r: u7\n}\n")
            } else {
                field(ty, range)
            };
            let (d, _) = diagnosed(&src);
            assert_eq!(d.code, "E0424", "{}", d);
            let got = d.fixes.first().map(|f| f.edits[0].replacement.as_str());
            match (ty, fix) {
                ("u4", _) => assert_eq!(got, Some("bool"), "{}", d),
                (_, expected) => assert_eq!(got, expected, "{} in {}: {}", ty, range, d),
            }
        }
    }

    /// Every number of `@layout`, `@bytes` and `@bits` is a `LayoutNum`, at most the bytes a
    /// value may hold, and a bit number at most the 128 bits of the widest backing: one past
    /// them is an error that says so, the same on every host, and a fix whose numbers would pass
    /// them is not offered. The fuzzer found the first two as panics of `mn check` (2026-11-15
    /// and 11-17), near `usize::MAX`; the review of pull request 187 found numbers past `usize`
    /// that were never read, so a struct with one went unchecked or drew the wrong error.
    #[test]
    fn test_layout_numbers_past_their_limits_are_errors_with_no_fix_past_them() {
        const MAX: u128 = crate::structs::VALUE_BYTES;
        // A field 2^32 - 2 bytes wide ends a byte short of the most a value may hold.
        let huge = format!("  @bytes(0..{}) pub a: [2147483647]u16\n", MAX - 1);
        for (src, code) in [
            // An overlap whose fix would end past the limit (E0413).
            (
                layout(
                    "@layout(size: 8, align: 8)",
                    &format!("  @bytes(0..8) pub a: u64\n  @bytes(0..{}) pub b: u64\n", MAX),
                ),
                "E0413",
            ),
            // An array of more elements, or more bytes, than a value may hold has no size to
            // check against (E0417).
            (
                layout(
                    "@layout(size: 4, align: 4)",
                    "  @bytes(0..4) pub e: [9223372036854775808]u16\n",
                ),
                "E0417",
            ),
            (
                layout(
                    "@layout(size: 4, align: 4)",
                    "  @bytes(0..4) pub e: [2147483648]u16\n",
                ),
                "E0417",
            ),
            // Past the huge field: a field with no range (E0412), an empty range (E0414) and one
            // of the wrong width (E0415), each of whose fixes would end past the limit.
            (
                layout(
                    "@layout(size: 8, align: 8)",
                    &format!("{}  pub b: u64\n", huge),
                ),
                "E0412",
            ),
            (
                layout(
                    "@layout(size: 8, align: 8)",
                    &format!("{}  @bytes({1}..{1}) pub b: u64\n", huge, MAX - 1),
                ),
                "E0414",
            ),
            (
                layout(
                    "@layout(size: 8, align: 8)",
                    &format!("{}  @bytes({}..{}) pub b: u64\n", huge, MAX - 1, MAX),
                ),
                "E0415",
            ),
            // A bitstruct's overlap whose fix would end past bit 128 (E0421).
            (
                String::from("module t\npub bitstruct B : u16 {\n  @bits(0..8) pub a: u8\n  @bits(0..128) pub b: u8\n}\n"),
                "E0421",
            ),
            // Numbers past the limits, on every host (E0223, E0423): the review's `size` past
            // `usize`, which was never read and left the struct unchecked, one a byte past the
            // limit, which a host of 64 bits held, a `@bytes` past `usize`, which drew E0412
            // with a fix that wrote a second `@bytes`, and a `@bits` past it, which read as the
            // empty range 0..0.
            (
                layout(
                    "@layout(size: 18446744073709551616, align: 8)",
                    "  @bytes(0..8) pub a: u64\n",
                ),
                "E0223",
            ),
            (
                layout(
                    "@layout(size: 4294967296, align: 8)",
                    "  @bytes(0..8) pub a: u64\n",
                ),
                "E0223",
            ),
            (
                layout(
                    "@layout(size: 8, align: 8)",
                    "  @bytes(0..18446744073709551616) pub a: u64\n",
                ),
                "E0223",
            ),
            (
                String::from("module t\npub bitstruct B : u8 {\n  @bits(0..18446744073709551616) pub a: u8\n}\n"),
                "E0423",
            ),
            (
                String::from("module t\npub bitstruct B : u8 {\n  @bits(0..129) pub a: u8\n}\n"),
                "E0423",
            ),
        ] {
            let (d, _) = diagnosed(&src);
            assert_eq!((d.code, d.fixes.len()), (code, 0), "{}\n{}", d, src);
        }
        // The array's error says what holds it back, and what to do.
        let (d, _) = diagnosed(&layout(
            "@layout(size: 4, align: 4)",
            "  @bytes(0..4) pub e: [2147483648]u16\n",
        ));
        assert!(
            d.message.contains("more than the 4294967295 bytes")
                && d.message
                    .contains("keep fewer elements, or split the field"),
            "{}",
            d.message
        );
        // Where the numbers fit, the fix is offered as before.
        let (d, _) = diagnosed(&layout(
            "@layout(size: 8, align: 8)",
            "  @bytes(0..8) pub a: u64\n  @bytes(0..8) pub b: u64\n",
        ));
        assert_eq!(
            (d.code, d.fixes[0].edits[0].replacement.as_str()),
            ("E0413", "@bytes(8..16)")
        );
    }

    /// Layout numbers are read as every integer literal is: in hexadecimal or binary, with digits
    /// grouped by `_`, where the review of pull request 187 found `[0x10]u8` and `[4_0]u8` of
    /// no known size (E0417). A `@layout` without its `size` or `align`, or with a number that
    /// is no literal, is an error (E0426), where the review found `@layout(size: 8)` passing
    /// unchecked; so is a `@bits` that is no range, and a `@bytes` that is none draws E0412
    /// with a fix that replaces it, not one that writes a second.
    #[test]
    fn test_layout_numbers_are_read_as_literals_and_malformed_ones_are_errors() {
        for fields in [
            "  @bytes(0..16) pub a: [0x10]u8\n",
            "  @bytes(0..16) pub a: [1_6]u8\n",
            "  @bytes(0x0..0b10000) pub a: [16]u8\n",
        ] {
            let src = layout("@layout(size: 0x10, align: 0b1)", fields);
            assert!(
                check_source(&src).is_ok(),
                "{}: {:?}",
                src,
                check_source(&src).err()
            );
        }
        for attr in [
            "@layout(size: 8)",
            "@layout(align: 8)",
            "@layout(size: eight, align: 8)",
            "@layout(size: 8, align: 8, pad: 1)",
            "@layout",
        ] {
            let (d, text) = diagnosed(&layout(attr, "  @bytes(0..8) pub a: u64\n"));
            assert_eq!((d.code, text.as_str()), ("E0426", attr), "{}", d);
        }
        for field in ["  @bits(3) pub a: u8\n", "  pub a: u8\n"] {
            let src = format!("module t\npub bitstruct B : u8 {{\n{}}}\n", field);
            assert_eq!(diagnosed(&src).0.code, "E0426", "{}", src);
        }
        let src = layout(
            "@layout(size: 8, align: 4)",
            "  @bytes(0..4) pub a: u32\n  @bytes(4) pub b: u32\n",
        );
        let (d, _) = diagnosed(&src);
        assert_eq!(d.code, "E0412");
        let edit = &d.fixes[0].edits[0];
        assert_eq!(
            (
                &src[edit.span.start..edit.span.end],
                edit.replacement.as_str()
            ),
            ("@bytes(4)", "@bytes(4..8)")
        );
        let fixed = mente_syntax::diagnostics::apply_fix(&src, &d.fixes[0]).unwrap();
        assert!(check_source(&fixed).is_ok(), "{}", fixed);
    }

    /// A layout states each of its numbers once: a second `@layout`, `@bytes` or `@bits` is
    /// `E0427`, at the second. The review of pull request 187 found `@bytes(0..8) @bytes(0..eight)`
    /// drawing E0412 at the valid first, whose fix rewrote it with its own text: the parser read
    /// the last `@bytes` and the layout check the first. Both read the first now.
    #[test]
    fn test_a_layout_attribute_written_twice_is_e0427() {
        for (src, second) in [
            (
                layout(
                    "@layout(size: 8, align: 8)\n@layout(size: 16, align: 8)",
                    "  @bytes(0..8) pub a: u64\n",
                ),
                "@layout(size: 16, align: 8)",
            ),
            (
                layout(
                    "@layout(size: 8, align: 8)",
                    "  @bytes(0..8) @bytes(0..eight) pub a: u64\n",
                ),
                "@bytes(0..eight)",
            ),
            (
                String::from("module t\npub bitstruct B : u8 {\n  @bits(0..4) @bits(0..8) pub a: u4\n  @bits(4..8) pub b: u4\n}\n"),
                "@bits(0..8)",
            ),
        ] {
            let (d, text) = diagnosed(&src);
            assert_eq!((d.code, text.as_str()), ("E0427", second), "{}", d);
            assert!(
                d.message.ends_with(&format!(
                    "`{}`; a layout states each of its numbers once, so keep one",
                    second
                )),
                "{}",
                d.message
            );
        }
        // The first is the one read: a valid first and an unreadable second is still E0427, not
        // the E0412 of a range the check could not read.
        let src = layout(
            "@layout(size: 8, align: 8)",
            "  @bytes(0..8) @bytes(0..eight) pub a: u64\n",
        );
        assert_eq!(check_source(&src).unwrap_err().len(), 1);
    }

    /// A layout number counts bytes, bits or elements, which have no type, so it takes no suffix:
    /// the review of pull request 187 found `size: 8_banana` and `@bytes(0..300_u8)` read as 8 and
    /// 300. Each place a layout number stands says so, and names the number to write; one with no
    /// suffix, in hexadecimal or with `_` between its digits, stays a number.
    #[test]
    fn test_a_layout_number_takes_no_suffix() {
        let why = |literal: &str, suffix: &str, n: u32| {
            format!(
                "`{}` has the suffix `{}`, and a layout number counts bytes, bits or elements, which have no type: write `{}`",
                literal, suffix, n
            )
        };
        for (src, code, literal, suffix, n) in [
            (
                layout(
                    "@layout(size: 8_banana, align: 8)",
                    "  @bytes(0..8) pub a: u64\n",
                ),
                "E0426",
                "8_banana",
                "banana",
                8,
            ),
            (
                layout(
                    "@layout(size: 8, align: 8)",
                    "  @bytes(0..8_u8) pub a: u64\n",
                ),
                "E0412",
                "8_u8",
                "u8",
                8,
            ),
            (
                String::from("module t\npub bitstruct B : u8 {\n  @bits(0..8_u8) pub a: u8\n}\n"),
                "E0426",
                "8_u8",
                "u8",
                8,
            ),
            (
                layout(
                    "@layout(size: 4, align: 1)",
                    "  @bytes(0..4) pub a: [4_u8]u8\n",
                ),
                "E0417",
                "4_u8",
                "u8",
                4,
            ),
        ] {
            let (d, _) = diagnosed(&src);
            assert_eq!(d.code, code, "{}", d);
            assert!(
                d.message.contains(&why(literal, suffix, n)),
                "{}",
                d.message
            );
        }
        assert_eq!(
            mente_syntax::int_literal("300_u8"),
            Some((300, Some("u8".into())))
        );
        assert_eq!(
            mente_syntax::int_literal("0xFF_u8"),
            Some((255, Some("u8".into())))
        );
        assert_eq!(mente_syntax::int_literal_value("300_u8"), None);
        assert_eq!(mente_syntax::int_literal_value("0x10"), Some(16));
        assert_eq!(mente_syntax::int_literal_value("4_0"), Some(40));
    }
}

#[cfg(test)]
mod overflow_traps;
