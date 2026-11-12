//! Checking a module item by item, for `mn check --watch` (`ROADMAP.md`, M1 checkpoint (a)).
//!
//! The Tier-1 latency spike found that an edit must re-parse and re-check only the item it
//! falls in to meet the 5 ms target, and that re-parsing the whole module misses it
//! ([`tier1-latency-spike`](../../docs/design-review.md#tier1-latency-spike)). A [`Session`]
//! therefore splits the module into its top-level items, parses and checks each on its own
//! with spans relative to the item, and keeps the results by the item's text: an item whose
//! text an edit leaves alone keeps them wherever it moves. A function's names, types and Tier 1
//! are checked again only when its text changes, or when a function's name, signature or
//! header does: Tier 1 reads its callees' refinements and contracts.
//! Its diagnostics are those `check_source` reports for the whole module, in the same order.

use std::collections::HashMap;

use mente_syntax::diagnostics::{in_source_order, Diagnostic};
use mente_syntax::tree::FnSyntax;
use mente_syntax::{
    lex, parse_bodies, parse_module, split_items, ItemDecl, ItemStart, Span, TokenKind,
};

use crate::check::{self, Signature};
use crate::structs::{clashes, Structs};
use crate::{check_source, names, tier1, verify_function_contracts, wide};

/// What parsing one item gives, every span relative to the item.
#[derive(Clone, Debug, Default)]
struct Parsed {
    /// The declaration parser's error, with its rank: a lexical error 0, a lifetime 1, the rest
    /// 2, the order in which `parse_module` looks for them across the whole module.
    error: Option<(u8, Diagnostic)>,
    /// The function the item declares.
    function: Option<Function>,
    /// The item declares something other than one function, which the session checks with the
    /// whole module.
    other: bool,
    /// The item starts with a `module` line, which only the first item may hold.
    header: bool,
}

#[derive(Clone, Debug)]
struct Function {
    name: String,
    /// Where its name is declared, relative to the item.
    name_span: Span,
    /// The declaration checks' error.
    declaration: Option<Diagnostic>,
    /// The syntax tree of its body, when it has one.
    syntax: Option<FnSyntax>,
    /// What the body parser reported.
    body: Vec<Diagnostic>,
    /// How it routes its clauses outside the Tier-1 fragment.
    routing: tier1::Routing,
}

/// What every function's check reads of one function: its name, its signature, and the header
/// Tier 1 reads, its refinements and contracts.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    name: String,
    signature: Option<Signature>,
    header: Option<String>,
}

#[derive(Clone, Debug)]
struct Entry {
    parsed: Parsed,
    /// The names and types checks' and Tier 1's diagnostics, with the context generation they
    /// hold for.
    checked: Option<(u64, Vec<Diagnostic>)>,
}

/// What the last update did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    /// Items parsed again.
    pub parsed: usize,
    /// Functions whose names, types and Tier 1 were checked again.
    pub checked: usize,
    /// Whether the whole module was checked, since it declares more than functions.
    pub whole: bool,
}

/// One module, checked item by item across edits.
#[derive(Debug, Default)]
pub struct Session {
    src: String,
    items: Vec<(ItemStart, String)>,
    cache: HashMap<String, Entry>,
    /// What every function's check reads of each function.
    context: Vec<Context>,
    generation: u64,
    diagnostics: Vec<Diagnostic>,
    work: Work,
}

impl Session {
    pub fn new(src: &str) -> Session {
        let mut s = Session::default();
        s.update(src);
        s
    }

    /// The module's text now.
    pub fn source(&self) -> &str {
        &self.src
    }

    /// What `check_source` reports for the module, in its order.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// What the last update parsed and checked again.
    pub fn work(&self) -> Work {
        self.work
    }

    /// Replaces the bytes `start..end` of the module's text with `text`.
    pub fn edit(&mut self, start: usize, end: usize, text: &str) {
        let mut src = self.src.clone();
        src.replace_range(start..end, text);
        self.update(&src);
    }

    /// Takes the module's new text and checks again what it changed.
    pub fn update(&mut self, src: &str) {
        self.src = src.to_string();
        self.work = Work::default();
        let starts = split_items(src);
        let mut items = Vec::new();
        for (i, at) in starts.iter().enumerate() {
            let end = starts.get(i + 1).map_or(src.len(), |next| next.offset);
            items.push((*at, src[at.offset..end].to_string()));
        }
        let mut cache = HashMap::new();
        for (_, text) in &items {
            if cache.contains_key(text) {
                continue;
            }
            let entry = match self.cache.remove(text) {
                Some(e) => e,
                None => {
                    self.work.parsed += 1;
                    Entry {
                        parsed: parse_item(text),
                        checked: None,
                    }
                }
            };
            cache.insert(text.clone(), entry);
        }
        self.items = items;
        self.cache = cache;
        self.diagnostics = self.check();
    }

    fn check(&mut self) -> Vec<Diagnostic> {
        let entries: Vec<&Parsed> = self
            .items
            .iter()
            .filter_map(|(_, text)| self.cache.get(text).map(|e| &e.parsed))
            .collect();
        // A `module` line after the first item is no declaration, which the whole module's
        // check reports (`E0111`), though the item parsed alone is a module of its own.
        if entries.iter().any(|p| p.other) || entries.iter().skip(1).any(|p| p.header) {
            self.work.whole = true;
            return check_source(&self.src).err().unwrap_or_default();
        }
        // `parse_module` stops at its first error, lexical errors before lifetimes before the rest.
        let first_error = self
            .items
            .iter()
            .filter_map(|(at, text)| {
                let e = self.cache.get(text)?.parsed.error.as_ref()?;
                Some((e.0, at.offset, relocate(&e.1, *at)))
            })
            .min_by_key(|(rank, offset, _)| (*rank, *offset));
        if let Some((_, offset, d)) = first_error {
            // An item's error, before another item: the whole module's parser reads the same
            // item within its own tokens, but where they end it names the next item's first
            // token, which this item's text alone cannot, so the session asks the whole
            // module, whose check stops at this error, cheaply. The last item's end is the
            // file's end for both.
            let last = self.items.last().map_or(0, |(at, _)| at.offset);
            if offset < last {
                self.work.whole = true;
                return check_source(&self.src).err().unwrap_or_default();
            }
            return vec![d];
        }
        let functions: Vec<&Function> = self
            .items
            .iter()
            .filter_map(|(_, text)| self.cache.get(text)?.parsed.function.as_ref())
            .collect();
        let context: Vec<Context> = functions
            .iter()
            .map(|f| Context {
                name: f.name.clone(),
                signature: f.syntax.as_ref().map(Signature::of),
                header: f.syntax.as_ref().map(tier1::header),
            })
            .collect();
        let callees: Vec<Option<tier1::Callee>> = functions
            .iter()
            .map(|f| f.syntax.as_ref().map(|syntax| tier1::Callee { syntax }))
            .collect();
        if context != self.context {
            self.context = context;
            self.generation += 1;
        }
        let names: Vec<String> = self.context.iter().map(|c| c.name.clone()).collect();
        let signatures: Vec<Option<Signature>> =
            self.context.iter().map(|c| c.signature.clone()).collect();
        // Check again each function whose results are stale, once per text.
        let mut fresh: HashMap<&str, Vec<Diagnostic>> = HashMap::new();
        for (_, text) in &self.items {
            let Some(entry) = self.cache.get(text) else {
                continue;
            };
            let Some(f) = &entry.parsed.function else {
                continue;
            };
            let Some(syntax) = f.syntax.as_ref().filter(|_| f.body.is_empty()) else {
                continue;
            };
            let current = entry
                .checked
                .as_ref()
                .is_some_and(|(g, _)| *g == self.generation);
            if current || fresh.contains_key(text.as_str()) {
                continue;
            }
            self.work.checked += 1;
            // A module of functions alone has no structs; one with any is checked whole.
            let structs = Structs::default();
            let (resolution, mut found) = names::resolve(syntax, &names, &structs);
            let (typing, typed) = check::check(syntax, &resolution, &signatures, &structs);
            found.extend(typed);
            found.extend(wide::check(syntax, &resolution, &typing, text));
            if found.is_empty() {
                let function = tier1::Function {
                    syntax,
                    resolution: &resolution,
                    typing: &typing,
                    routing: f.routing,
                    structs: &structs,
                };
                found.extend(tier1::check(&function, &callees, text).diagnostics);
            }
            fresh.insert(text, found);
        }
        drop(callees);
        let fresh: Vec<(String, Vec<Diagnostic>)> = fresh
            .into_iter()
            .map(|(text, found)| (text.to_string(), found))
            .collect();
        for (text, found) in fresh {
            if let Some(entry) = self.cache.get_mut(&text) {
                entry.checked = Some((self.generation, found));
            }
        }
        let mut declarations = Vec::new();
        // A name declared twice, which the whole module's check reports at the second name.
        let named: Vec<(String, Span)> = self
            .items
            .iter()
            .filter_map(|(at, text)| {
                let f = self.cache.get(text)?.parsed.function.as_ref()?;
                Some((f.name.clone(), relocate_span(f.name_span, *at)))
            })
            .collect();
        let mut bodies = clashes(named.iter().map(|(n, s)| (n.as_str(), *s, "a function")));
        for (at, text) in &self.items {
            let Some(entry) = self.cache.get(text) else {
                continue;
            };
            let Some(f) = &entry.parsed.function else {
                continue;
            };
            declarations.extend(f.declaration.iter().map(|d| relocate(d, *at)));
            bodies.extend(f.body.iter().map(|d| relocate(d, *at)));
            // Only a function whose body parsed cleanly was checked.
            if let Some((_, found)) = &entry.checked {
                bodies.extend(found.iter().map(|d| relocate(d, *at)));
            }
        }
        declarations.extend(bodies);
        in_source_order(&mut declarations);
        declarations
    }
}

/// Parses one item's text on its own.
fn parse_item(text: &str) -> Parsed {
    // Its first token, past doc comments, is `module`.
    let header = lex(text)
        .0
        .iter()
        .find(|t| !matches!(t.kind, TokenKind::DocComment(_)))
        .is_some_and(|t| t.kind == TokenKind::Module);
    let module = match parse_module(text) {
        Ok(m) => m,
        Err(d) => {
            let rank = if !lex(text).1.is_empty() {
                0
            } else if d.code == "E0120" {
                1
            } else {
                2
            };
            return Parsed {
                error: Some((rank, d)),
                header,
                ..Parsed::default()
            };
        }
    };
    match module.items.as_slice() {
        [] => Parsed {
            header,
            ..Parsed::default()
        },
        [ItemDecl::Fn(f)] => {
            let parsed = parse_bodies(text, &module).into_iter().next();
            let (syntax, body) = parsed.map_or((None, Vec::new()), |p| (p.syntax, p.errors));
            Parsed {
                function: Some(Function {
                    name: f.name.clone(),
                    name_span: f.name_span,
                    declaration: verify_function_contracts(f).err(),
                    syntax,
                    body,
                    routing: tier1::Routing::of(&f.attrs),
                }),
                header,
                ..Parsed::default()
            }
        }
        // An item of more than one function is checked with the whole module, which holds
        // any `module` line too.
        _ => Parsed {
            other: true,
            ..Parsed::default()
        },
    }
}

/// A diagnostic of an item, with its spans moved from the item's start to the module's.
/// A span of an item, relative to the item, at its place in the module.
fn relocate_span(s: Span, at: ItemStart) -> Span {
    Span {
        start: s.start + at.offset,
        end: s.end + at.offset,
        line: s.line + at.line - 1,
        col: s.col,
    }
}

fn relocate(d: &Diagnostic, at: ItemStart) -> Diagnostic {
    let shift = |s: Span| relocate_span(s, at);
    let mut d = d.clone();
    d.primary = shift(d.primary);
    for label in &mut d.related {
        label.span = shift(label.span);
    }
    for fix in &mut d.fixes {
        for edit in &mut fix.edits {
            edit.span = shift(edit.span);
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;

    fn whole(src: &str) -> Vec<Diagnostic> {
        check_source(src).err().unwrap_or_default()
    }

    fn mn_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                mn_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "mn") {
                out.push(path);
            }
        }
    }

    /// Every module in the repository: item by item, a session reports exactly what the whole
    /// module's check does.
    #[test]
    fn test_a_session_reports_what_the_whole_module_check_does() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        for dir in [
            "examples",
            "conformance",
            "benchmark/probe",
            "benchmark/prelim",
        ] {
            mn_files(&root.join(dir), &mut files);
        }
        assert!(files.len() > 100, "{}", files.len());
        let mut item_by_item = 0;
        for file in &files {
            let src = fs::read_to_string(file).unwrap_or_default();
            let session = Session::new(&src);
            assert_eq!(
                session.diagnostics(),
                whole(&src).as_slice(),
                "{}",
                file.display()
            );
            if !session.work().whole {
                item_by_item += 1;
            }
        }
        assert!(item_by_item > 100, "{}", item_by_item);
    }

    const MODULE: &str = "module m\n\npub fn id(x: u8) -> u8 = x\n\n/// Doubles.\npub fn twice(x: u8) -> u16 {\n  val y: u16 = x\n  y + y\n}\n\npub fn main() -> u16 = twice(id(3))\n";

    /// Applies an edit to a session and to the text, and checks that they agree.
    fn edited(session: &mut Session, from: &str, to: &str) -> Work {
        let start = session.source().find(from).unwrap_or(0);
        session.edit(start, start + from.len(), to);
        let src = session.source().to_string();
        assert_eq!(session.diagnostics(), whole(&src).as_slice(), "{}", src);
        session.work()
    }

    #[test]
    fn test_an_edit_parses_and_checks_again_only_what_it_changes() {
        let mut s = Session::new(MODULE);
        assert!(s.diagnostics().is_empty());
        assert_eq!(
            s.work(),
            Work {
                parsed: 4,
                checked: 3,
                whole: false
            }
        );
        // A body edit: one item parsed and checked again.
        let w = edited(&mut s, "y + y", "y + z");
        assert_eq!(
            w,
            Work {
                parsed: 1,
                checked: 1,
                whole: false
            }
        );
        assert_eq!(s.diagnostics()[0].code, "E0201");
        assert_eq!(s.diagnostics()[0].primary.line, 8);
        let w = edited(&mut s, "y + z", "y + y");
        assert_eq!((w.parsed, w.checked), (1, 1));
        assert!(s.diagnostics().is_empty());
        // The same text again: nothing to do.
        let src = s.source().to_string();
        s.update(&src);
        assert_eq!(s.work(), Work::default());
        // A line added above moves every item after it, which keep their results.
        let w = edited(&mut s, "module m\n", "module m\n\n");
        assert_eq!((w.parsed, w.checked), (1, 0));
        // A comment above a function belongs to it.
        let w = edited(&mut s, "\npub fn id", "\n// the identity\npub fn id");
        assert_eq!((w.parsed, w.checked), (1, 1));
        // A signature changes what callers see: every function is checked again.
        let w = edited(
            &mut s,
            "pub fn id(x: u8) -> u8 = x",
            "pub fn id(x: u8) -> bool = true",
        );
        assert_eq!((w.parsed, w.checked), (1, 3));
        assert_eq!(s.diagnostics()[0].code, "E0203");
        // A new function, and one removed.
        let w = edited(
            &mut s,
            "\npub fn main()",
            "\npub fn extra() -> u8 = 1\n\npub fn main()",
        );
        assert_eq!(w.parsed, 1);
        edited(&mut s, "pub fn extra() -> u8 = 1\n\n", "");
    }

    #[test]
    fn test_errors_come_in_the_order_the_whole_module_check_gives() {
        // Declaration errors in two items: a later lexical error, or a later lifetime, wins
        // over an earlier parse error. (A parameter written wrong is the body parser's error,
        // so a construct of a later milestone stands for the declaration parser's.)
        let mut d = Session::new(MODULE);
        edited(&mut d, "pub fn id(x: u8) -> u8 = x", "import foo");
        let parse = d.diagnostics()[0].code;
        assert_eq!(parse, "E0109");
        edited(&mut d, "twice(id(3))", "twice(id(3)) $");
        assert_eq!(d.diagnostics()[0].code, "E0105");
        edited(&mut d, " $", "");
        assert_eq!(d.diagnostics()[0].code, parse);
        edited(&mut d, "pub fn main()", "pub fn main<'a>()");
        assert_eq!(d.diagnostics()[0].code, "E0120");
        let mut s = Session::new(MODULE);
        // A parse error in one item and a lexical error in a later one: the lexical error wins.
        edited(&mut s, "val y: u16 = x", "val y: u16 = (x");
        edited(
            &mut s,
            "pub fn main() -> u16 = twice(id(3))",
            "pub fn main() -> u16 = twice(id(3)) $",
        );
        assert_eq!(s.diagnostics().len(), 1);
        assert_eq!(s.diagnostics()[0].code, "E0105");
        edited(&mut s, " $", "");
        // A lifetime outranks a later parse error and an earlier one alike.
        edited(
            &mut s,
            "pub fn id(x: u8) -> u8 = x",
            "pub fn id<'a>(x: u8) -> u8 = x",
        );
        assert_eq!(s.diagnostics()[0].code, "E0120");
        edited(&mut s, "<'a>", "");
        assert_eq!(s.diagnostics()[0].code, "E0111");
        edited(&mut s, "(x\n", "x\n");
        assert!(s.diagnostics().is_empty());
        // Declaration errors and body errors come in the order of the source, and fixes move
        // with their item.
        edited(&mut s, "pub fn main() -> u16 = twice(id(3))", "@total\npub fn main() -> u16 {\n  var i: u8 = 0\n  while i < 3 {\n    i += 1\n  }\n  twice(id(i))\n}");
        edited(&mut s, "y + y", "zz");
        assert!(s.diagnostics().iter().any(|d| d.code == "E0501"));
        let starts: Vec<usize> = s.diagnostics().iter().map(|d| d.primary.start).collect();
        assert!(starts.windows(2).all(|w| w[0] <= w[1]), "{:?}", starts);
        let fix = s
            .diagnostics()
            .iter()
            .find(|d| d.code == "E0201")
            .map(|d| d.fixes.clone());
        assert!(fix.is_some());
    }

    /// Tier 1 reads a callee's refinements and contracts, which its signature does not hold: an
    /// edit to them proves every caller again.
    #[test]
    fn test_a_callee_contract_edit_proves_its_callers_again() {
        let src = "module m\n\npub fn pick(lo: u8, hi: u8) -> u8\n  requires lo < hi\n= lo\n\npub fn caller(n: u8) -> u8 {\n  guard n > 0 else {\n    return 0\n  }\n  pick(0, n)\n}\n";
        let mut s = Session::new(src);
        assert!(s.diagnostics().is_empty());
        // A stronger `requires`: the caller's call no longer proves it.
        let w = edited(&mut s, "requires lo < hi", "requires lo + 1 < hi");
        assert_eq!((w.parsed, w.checked), (1, 2));
        assert_eq!(s.diagnostics()[0].code, "E0504");
        assert_eq!(s.diagnostics()[0].primary.line, 11);
        // A refinement of a parameter, then of the result.
        edited(&mut s, "requires lo + 1 < hi", "requires lo < hi");
        assert!(s.diagnostics().is_empty());
        let w = edited(&mut s, "hi: u8)", "hi: u8[2..9])");
        assert_eq!(w.checked, 2);
        assert_eq!(s.diagnostics()[0].code, "E0502");
        edited(&mut s, "hi: u8[2..9])", "hi: u8)");
        edited(
            &mut s,
            "pick(0, n)\n}",
            "val r: u8[0..1] = pick(0, n)\n  r\n}",
        );
        assert_eq!(s.diagnostics()[0].code, "E0502");
        let w = edited(&mut s, "-> u8\n  requires", "-> u8[0..1]\n  requires");
        assert_eq!(w.checked, 2);
        // The caller is proven now; the callee's own body is not.
        assert_eq!(s.diagnostics().len(), 1);
        assert_eq!(s.diagnostics()[0].primary.line, 5);
        // A body edit of the callee leaves its callers alone.
        let w = edited(&mut s, "= lo\n", "= 0\n");
        assert_eq!((w.parsed, w.checked), (1, 1));
        assert!(s.diagnostics().is_empty());
    }

    #[test]
    fn test_a_module_line_after_the_first_item_is_reported_as_the_whole_check_does() {
        for src in [
            "module c\nmodule c\n",
            "module m\n\npub fn f() -> u8 = 1\nmodule n\n",
            "module m\n\npub fn f() -> u8 = 1\nmodule n pub fn g() -> u8 = 2\n",
        ] {
            let s = Session::new(src);
            let whole = check_source(src).err().unwrap_or_default();
            assert!(!whole.is_empty(), "{}", src);
            let codes = |d: &[Diagnostic]| d.iter().map(|d| d.code).collect::<Vec<_>>();
            assert_eq!(codes(s.diagnostics()), codes(&whole), "{}", src);
            assert!(s.work().whole, "{}", src);
        }
    }

    /// Code written without indentation splits where the whole module's check reads its
    /// declarations, so a session reports what that check does.
    #[test]
    fn test_unindented_code_is_checked_as_the_whole_module_is() {
        for src in [
            "module m\n\npub fn h(x: i32) -> i32 {\n  val y = x\ny\n}\n",
            "module m\n\npub fn h(x: i32) -> i32 {\nvar y: i32 = x\nwhile y > 0 {\ny -= 1\n}\ny\n}\n",
            "module m\n\npub fn h(x: i32) -> i32 =\nx + 1\n",
            // A function without its parameter list, split over two lines.
            "module x\nfn\nm\n",
            // A list open at the end of an item, and a module line without its name.
            "module m\n\npub fn f(x: u8 {\n}\n",
            "module\nfn",
            // A return type whose bracket closes nothing, or is never closed, before an item.
            "module m\n\nfn r() -> ]\nfn\n",
            "module m\n\npub fn f() -> 20]i32 = 0\npub fn g() -> u8 = 1\n",
            "module m\n\npub fn f() -> [4 = 0\npub fn g() -> u8 = 1\n",
            // An expression body or a contract that would go on past a newline, before a line
            // that begins an item: a declaration, or a comment in the first column.
            "module m\n\nfn t() =\npub fn g() -> i32 = 0\n",
            "module m\n\npub fn f(x: i32) -> i32 = x +\n// note\n  1\n",
            "module m\n\npub fn f(x: i32) -> i32\n  requires x >\npub fn g() -> i32 = 0\n",
            // A contract whose bracket another closes, before an item (the fuzzer's finding 18).
            "module m\n\nfn e()->2\nrequires(}\npub",
        ] {
            let s = Session::new(src);
            let whole = check_source(src).err().unwrap_or_default();
            assert_eq!(s.diagnostics(), whole.as_slice(), "{}", src);
        }
    }

    /// A parse error in an item before another sends the session to the whole module's check,
    /// whose words for the item's end name the next item's first token; the last item's error
    /// is the session's own, since its end is the file's end for both.
    #[test]
    fn test_a_parse_error_before_another_item_sends_the_session_whole() {
        for (src, whole) in [
            ("module\nfn", true),
            ("module m\npub fn f(\n", false),
            ("module m\npub fn fn() -> u8 = 1\npub fn g(\n", true),
            // A second module line, without its name, that fails to parse on its own.
            ("module p\nmodule", true),
            // An error inside an item whose words name the item's end (the fuzzer's finding 19).
            ("module m\nfn t()->8 from\npub", true),
        ] {
            let s = Session::new(src);
            let expected = check_source(src).err().unwrap_or_default();
            assert!(!expected.is_empty(), "{}", src);
            assert_eq!(s.diagnostics(), expected.as_slice(), "{}", src);
            assert_eq!(s.work().whole, whole, "{}", src);
        }
    }

    #[test]
    fn test_a_module_with_more_than_functions_is_checked_whole() {
        let mut s = Session::new(MODULE);
        let w = edited(
            &mut s,
            "\npub fn main()",
            "\npub struct P {\n  x: u8\n}\n\npub fn main()",
        );
        assert!(w.whole);
        edited(&mut s, "pub struct P {\n  x: u8\n}\n\n", "");
        assert!(!s.work().whole);
    }
}

#[cfg(test)]
mod timing {
    use std::time::Instant;

    use super::*;

    /// Prints how long a session takes to re-check a body edit in a module of 200 functions,
    /// against checking the whole module again, names, types and Tier 1 included.
    #[test]
    #[ignore]
    fn watch_recheck_sketch() {
        let mut src = String::from("module m\n");
        for i in 0..200 {
            src.push_str(&format!(
                "\npub fn f{i}(xs: [8]i32, n: usize) -> i64 {{\n  var total: i64 = 0\n  var k: usize = 0\n  while k < n and k < 8 {{\n    if xs[k] > 0 {{\n      total += xs[k]\n    }}\n    k += 1\n  }}\n  total\n}}\n"
            ));
        }
        let mut session = Session::new(&src);
        let mut edits = Vec::new();
        let mut wholes = Vec::new();
        for round in 0..200 {
            let i = (round * 37) % 200;
            let from = format!("pub fn f{i}(xs: [8]i32, n: usize) -> i64 {{\n  var total: i64 = 0");
            let to =
                format!("pub fn f{i}(xs: [8]i32, n: usize) -> i64 {{\n  var total: i64 = {round}");
            let current = session.source().to_string();
            let Some(at) = current.find(&from) else {
                continue;
            };
            let start = at + from.len() - 1;
            let t0 = Instant::now();
            session.edit(start, start + 1, &round.to_string());
            edits.push(t0.elapsed().as_micros());
            let edited = session.source().to_string();
            let t1 = Instant::now();
            let _ = check_source(&edited);
            wholes.push(t1.elapsed().as_micros());
            let _ = to;
        }
        edits.sort_unstable();
        wholes.sort_unstable();
        let p = |v: &[u128], q: f64| v[((v.len() - 1) as f64 * q) as usize];
        println!(
            "edit re-check: median {} us, p99 {} us; whole module: median {} us, p99 {} us",
            p(&edits, 0.5),
            p(&edits, 0.99),
            p(&wholes, 0.5),
            p(&wholes, 0.99)
        );
    }
}
