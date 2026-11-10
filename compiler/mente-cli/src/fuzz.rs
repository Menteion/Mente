//! `mn fuzz`: the compiler's own fuzzer (`docs/design-brief.md`, the quality-system table; M1's
//! exit criterion 6, "fuzzing finds no crash"). It is written here, with no fuzzing crate, so
//! that it builds on stable Rust with the workspace's dependencies alone.
//!
//! - **Inputs** are mutations of a seed corpus, the repository's own `.mn` programs, at seven
//!   levels ([`Family`]): bytes, non-ASCII text, tokens, integer literals, lines and items,
//!   functions spliced from other seeds, and deep nesting. A mutant that checks joins the
//!   corpus, so later mutants start deeper in the compiler.
//! - **Every phase an input reaches runs** ([`Phases`]): the lexer and its dump, the
//!   declaration parser, the body parser and its dump, the blueprint, `check_source` and the
//!   rendering of its diagnostics, the typed dump, Tier 1, the incremental re-check of an edit,
//!   `check_program` with and without proofs; and for a module that checks, the lowering, the
//!   MIR dump, an object for each of the three targets, and each function whose parameters are
//!   all scalars on edge-value arguments, on the reference interpreter and on MIR, under tight
//!   limits.
//! - **The oracle is "no crash"**, never semantics, which the three-path comparison of
//!   generated programs owns. A [`Failure`] is a panic; a stack overflow or any other abnormal
//!   end of the process; a phase that takes longer than its budget; broken MIR, a lowering
//!   error or a codegen error from a module that checks; two runs of a phase that give
//!   different output; or an incremental re-check that reports otherwise than a whole check.
//!   Diagnostics, refusals and traps are correct behaviour on bad input.
//! - **Isolation**: the phases run in a worker process (`mn fuzz --worker`), on a thread whose
//!   stack is the 1 MiB `mn` gets on Windows, so a stack overflow found is one a user can hit.
//!   A worker that dies or stops answering is a crash or a hang of the last phase it named,
//!   and is started again.
//! - **Each failure is minimized** (delta debugging over lines, then tokens, then characters)
//!   and written to `--out` once, deduplicated by its phase and message.
//!
//! Everything is reproducible from `--seed`: a job's inputs depend only on the seed, the job's
//! number and the compiler's answers. The compiler takes `&str`, and `mn` refuses a file that
//! is not UTF-8 before lexing, so a byte mutation that breaks UTF-8 reaches the compiler as
//! U+FFFD, as an editor shows it. `MENTE_FUZZ_PLANT` (`panic`, `overflow` or `hang`) makes a
//! worker fail on any input holding [`PLANT_MARK`], for the fuzzer's own tests.

use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, Once};
use std::time::{Duration, Instant};

use mente_codegen::lower::{Lowered, Shape};
use mente_codegen::native;
use mente_interp::{Int, Value};
use mente_mir::cfg::{self, Memory, Num, Val};
use mente_sema::CheckedModule;
use mente_syntax::diagnostics::{render_json, render_text, Diagnostic, FileDiagnostics};
use mente_syntax::TokenKind;

/// The stack the phases run on: the 1 MiB the main thread of `mn` gets on Windows, the
/// smallest of the three platforms.
const STACK_BYTES: usize = 1 << 20;

/// How long one phase may take on one input before it counts as a hang.
const PHASE_SECONDS: u64 = 10;

/// How long past the phase budget a worker may stay silent before it is stopped.
const GRACE: Duration = Duration::from_secs(3);

/// The largest input a mutation leaves, in bytes; deep nesting may go up to [`MAX_NESTED`].
const MAX_INPUT: usize = 64 << 10;
const MAX_NESTED: usize = 512 << 10;

/// The most mutants that join a job's corpus.
const MAX_CORPUS: usize = 4096;

/// The most runs, and the longest time, that minimizing one failure may take; near or past
/// the campaign's deadline, the shorter time.
const MINIMIZE_TRIES: usize = 2000;
const MINIMIZE_TIME: Duration = Duration::from_secs(90);
const MINIMIZE_LATE: Duration = Duration::from_secs(20);

/// The most functions of one module the interpreters run.
const MAX_FUNCTIONS: usize = 16;

/// The limits of every interpreted call: tight, since only a crash matters. The reference
/// interpreter copies an array value at each assignment, so it gets fewer steps and smaller
/// arrays.
const MAX_STEPS: u64 = 100_000;
const MAX_DEPTH: usize = 64;
const REFERENCE_STEPS: u64 = 20_000;
const REFERENCE_ARRAY: u128 = 256;

/// The text a planted failure waits for (`MENTE_FUZZ_PLANT`).
pub const PLANT_MARK: &str = "fuzz-plant";

/// The directories of the seed corpus, from the repository's root.
const SEED_DIRS: [&str; 6] = [
    "examples",
    "conformance",
    "benchmark/probe",
    "benchmark/prelim",
    "perf/kernels",
    "compiler/mente-cli/tests/debuggers",
];

// -------------------------------------------------------------------------------------------
// The generator of random numbers
// -------------------------------------------------------------------------------------------

/// SplitMix64: small, fast, and the same on every platform, so a seed names one run.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number below `n`, or 0 when `n` is 0.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    /// True once in `n` times.
    pub fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }

    /// An item of a list that is not empty.
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items.get(self.below(items.len())).copied().unwrap_or("")
    }
}

// -------------------------------------------------------------------------------------------
// Mutations
// -------------------------------------------------------------------------------------------

/// The levels a mutation works at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// Bit flips, interesting bytes, insertions, deletions and copies of byte ranges; text
    /// that is no longer UTF-8 becomes U+FFFD.
    Bytes,
    /// Non-ASCII text: CJK, a byte order mark, zero-width and right-to-left marks, combining
    /// marks, emoji, homoglyphs, full-width punctuation, unusual line ends and control bytes.
    NonAscii,
    /// Delete, duplicate or swap tokens, or insert keywords, operators, types, attributes and
    /// other languages' idioms the lexer knows.
    Tokens,
    /// Integer literals moved to the edges: 0, 1, the type limits, 2^127, 2^128 - 1 and past
    /// them, long digit strings, suffixes and signs.
    Literals,
    /// Duplicate, delete, swap, move or truncate lines, and duplicate or delete items.
    Lines,
    /// An item or a block of lines from another seed.
    Splice,
    /// Deeply nested expressions, blocks, `if`s, `while`s, unary and binary chains, arrays,
    /// calls and indexing, to find a recursion without a limit.
    Nesting,
}

impl Family {
    pub const ALL: [Family; 7] = [
        Family::Bytes,
        Family::NonAscii,
        Family::Tokens,
        Family::Literals,
        Family::Lines,
        Family::Splice,
        Family::Nesting,
    ];
}

/// Every keyword the lexer knows.
pub const KEYWORDS: [&str; 46] = [
    "module",
    "import",
    "pub",
    "extern",
    "val",
    "var",
    "mut",
    "own",
    "const",
    "struct",
    "bitstruct",
    "enum",
    "trait",
    "type",
    "unit",
    "dyn",
    "fn",
    "return",
    "if",
    "else",
    "match",
    "guard",
    "while",
    "break",
    "continue",
    "for",
    "in",
    "parallel",
    "parallel_for",
    "actor",
    "with",
    "as",
    "comptime",
    "requires",
    "ensures",
    "invariant",
    "decreases",
    "raise",
    "catch",
    "and",
    "or",
    "not",
    "is",
    "none",
    "true",
    "false",
];

/// Every operator and punctuation token the lexer knows, and one token of each other kind: a
/// lifetime, a doc comment, a string, a float, a name and an integer.
pub const PUNCTUATION: [&str; 55] = [
    "@", "(", ")", "{", "}", "[", "]", ":", ",", ".", "..", "..=", "...", "->", "=>", "|>", "|",
    "&", "^", "~", "?", "?.", "??", "!", "=", "==", "!=", "<", "<=", ">", ">=", "<<", ">>", "+",
    "+=", "-", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=", "*", "/", "%", ";", "'a",
    "/// doc", "\"text\"", "1.5", "x", "0",
];

/// Types, attributes, built-ins and other languages' idioms, which the checker must reject or
/// accept without crashing.
const EXTRAS: [&str; 40] = [
    "i8",
    "u8",
    "i16",
    "u16",
    "i32",
    "u32",
    "i64",
    "u64",
    "i128",
    "u128",
    "isize",
    "usize",
    "bool",
    "[4]u8",
    "[0]i32",
    "u8[0..=5]",
    "i32[-3..3]",
    "@total",
    "@branchless",
    "@prove",
    "@kernel",
    "@layout(size: 8, align: 4)",
    "@bytes(0..4)",
    "@example",
    "todo(\"x\")",
    "unreachable(\"x\")",
    "let",
    "let mut",
    "&mut",
    "async",
    "await",
    "::<i32>",
    ".unwrap()",
    "<'a>",
    "x as u8",
    "return",
    "requires x > 0",
    "ensures result >= 0",
    "decreases n",
    "guard x < 10 else { return 0 }",
];

/// Non-ASCII text, written as escapes so that this file stays English-only.
const NON_ASCII: [&str; 20] = [
    "\u{4E2D}\u{6587}",
    "\u{8B8A}\u{6578}",
    "\u{3042}",
    "\u{D55C}",
    "\u{FEFF}",
    "\u{200B}",
    "\u{200D}",
    "\u{202E}",
    "e\u{301}",
    "\u{1F600}",
    "\u{FFFD}",
    "\u{A0}",
    "\u{2028}",
    "\u{430}",
    "\u{FF08}",
    "\u{3000}",
    "\r\n",
    "\r",
    "\t",
    "\0",
];

/// Bytes worth setting: the edges of UTF-8 and of ASCII, and the lexer's delimiters.
const INTERESTING_BYTES: [u8; 18] = [
    0x00, 0x7F, 0x80, 0xBF, 0xC0, 0xC3, 0xE2, 0xF0, 0xFF, b'{', b'}', b'(', b')', b'"', b'\\',
    b'\n', b'\'', b'@',
];

/// Integer literals at the edges of the integer types and past them.
const EDGES: [&str; 37] = [
    "0",
    "1",
    "2",
    "7",
    "8",
    "15",
    "16",
    "31",
    "32",
    "63",
    "64",
    "127",
    "128",
    "255",
    "256",
    "32767",
    "32768",
    "65535",
    "65536",
    "2147483647",
    "2147483648",
    "4294967295",
    "4294967296",
    "9223372036854775807",
    "9223372036854775808",
    "18446744073709551615",
    "18446744073709551616",
    "170141183460469231731687303715884105727",
    "170141183460469231731687303715884105728",
    "340282366920938463463374607431768211455",
    "340282366920938463463374607431768211456",
    "0x7F",
    "0xFF",
    "0xFFFFFFFFFFFFFFFF",
    "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
    "0b1",
    "1_000_000",
];

const SUFFIXES: [&str; 12] = [
    "_u8", "_i8", "_u16", "_i16", "_u32", "_i32", "_u64", "_i64", "_u128", "_i128", "_usize",
    "_isize",
];

/// How often each family of [`Family::ALL`] is chosen: tokens and literals most often, since
/// they keep a module closest to checking.
const WEIGHTS: [usize; 7] = [1, 1, 4, 2, 2, 1, 2];

/// Applies one to four mutations of the families `allowed` to `text`, `others` being the
/// corpus to splice from.
pub fn mutate(rng: &mut Rng, text: &str, others: &[String], allowed: &[Family]) -> String {
    let rounds = [1, 1, 1, 2, 2, 3, 4][rng.below(7)];
    let weighted: Vec<Family> = Family::ALL
        .into_iter()
        .zip(WEIGHTS)
        .filter(|(f, _)| allowed.contains(f))
        .flat_map(|(f, w)| std::iter::repeat_n(f, w))
        .collect();
    let mut out = text.to_string();
    for _ in 0..rounds {
        let Some(family) = weighted.get(rng.below(weighted.len())) else {
            break;
        };
        out = apply(*family, rng, &out, others);
    }
    out
}

/// One mutation of `family` on `text`.
pub fn apply(family: Family, rng: &mut Rng, text: &str, others: &[String]) -> String {
    let out = match family {
        Family::Bytes => bytes(rng, text),
        Family::NonAscii => non_ascii(rng, text),
        Family::Tokens => tokens(rng, text),
        Family::Literals => literals(rng, text),
        Family::Lines => lines(rng, text),
        Family::Splice => splice(rng, text, others),
        Family::Nesting => nesting(rng, text),
    };
    let cap = if family == Family::Nesting {
        MAX_NESTED
    } else {
        MAX_INPUT
    };
    if out.len() > cap.max(text.len()) {
        out[..floor(&out, cap.max(text.len()))].to_string()
    } else {
        out
    }
}

/// The largest character boundary of `text` at or below `at`.
fn floor(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// A character boundary of `text`, at random.
fn boundary(rng: &mut Rng, text: &str) -> usize {
    floor(text, rng.below(text.len() + 1))
}

/// `text` with the bytes `start..end` replaced by `with`; both ends are character boundaries.
fn replace(text: &str, start: usize, end: usize, with: &str) -> String {
    let mut out = String::with_capacity(text.len() + with.len());
    out.push_str(&text[..start]);
    out.push_str(with);
    out.push_str(&text[end..]);
    out
}

/// The spans of `text`'s tokens, each with whether it is an integer literal or an identifier.
fn token_spans(text: &str) -> Vec<(usize, usize, TokenKind)> {
    mente_syntax::tokenize(text)
        .into_iter()
        .filter(|t| t.kind != TokenKind::Eof && t.span.start < t.span.end)
        .filter(|t| text.is_char_boundary(t.span.start) && text.is_char_boundary(t.span.end))
        .filter(|t| t.span.end <= text.len())
        .map(|t| (t.span.start, t.span.end, t.kind))
        .collect()
}

fn bytes(rng: &mut Rng, text: &str) -> String {
    let mut b = text.as_bytes().to_vec();
    let len = b.len();
    match rng.below(5) {
        0 if len > 0 => {
            let i = rng.below(len);
            b[i] ^= 1 << rng.below(8);
        }
        1 if len > 0 => {
            let i = rng.below(len);
            b[i] = INTERESTING_BYTES[rng.below(INTERESTING_BYTES.len())];
        }
        0..=2 => {
            let at = rng.below(len + 1);
            let n = 1 + rng.below(8);
            let new: Vec<u8> = (0..n)
                .map(|_| {
                    if rng.one_in(2) {
                        INTERESTING_BYTES[rng.below(INTERESTING_BYTES.len())]
                    } else {
                        rng.next_u64() as u8
                    }
                })
                .collect();
            b.splice(at..at, new);
        }
        3 if len > 0 => {
            let at = rng.below(len);
            let end = (at + 1 + rng.below(16)).min(len);
            b.drain(at..end);
        }
        _ if len > 0 => {
            let at = rng.below(len);
            let end = (at + 1 + rng.below(32)).min(len);
            let copy = b[at..end].to_vec();
            let to = rng.below(len + 1);
            b.splice(to..to, copy);
        }
        _ => b.push(rng.next_u64() as u8),
    }
    String::from_utf8_lossy(&b).into_owned()
}

fn non_ascii(rng: &mut Rng, text: &str) -> String {
    let what = rng.pick(&NON_ASCII);
    let spans = token_spans(text);
    // Inside or beside a token most of the time, where a span may be cut in two.
    let at = if !spans.is_empty() && !rng.one_in(4) {
        let (start, end, _) = &spans[rng.below(spans.len())];
        match rng.below(3) {
            0 => *start,
            1 => *end,
            _ => floor(text, start + (end - start) / 2),
        }
    } else {
        boundary(rng, text)
    };
    if rng.one_in(4) && at < text.len() {
        // Replace the character there.
        let next = text[at..].chars().next().map_or(at, |c| at + c.len_utf8());
        replace(text, at, next, what)
    } else {
        replace(text, at, at, what)
    }
}

/// A keyword, an operator, a type, an attribute or an idiom.
fn spelling(rng: &mut Rng) -> &'static str {
    match rng.below(3) {
        0 => rng.pick(&KEYWORDS),
        1 => rng.pick(&PUNCTUATION),
        _ => rng.pick(&EXTRAS),
    }
}

fn tokens(rng: &mut Rng, text: &str) -> String {
    let spans = token_spans(text);
    if spans.is_empty() {
        let at = boundary(rng, text);
        return replace(text, at, at, spelling(rng));
    }
    let (start, end, _) = spans[rng.below(spans.len())].clone();
    match rng.below(6) {
        0 => replace(text, start, end, ""),
        1 => replace(text, end, end, &format!(" {}", &text[start..end])),
        2 => {
            let (s2, e2, _) = spans[rng.below(spans.len())].clone();
            let (a, b) = if s2 < start {
                ((s2, e2), (start, end))
            } else {
                ((start, end), (s2, e2))
            };
            if a.1 > b.0 || a == b {
                return replace(text, start, end, "");
            }
            let mut out = String::new();
            out.push_str(&text[..a.0]);
            out.push_str(&text[b.0..b.1]);
            out.push_str(&text[a.1..b.0]);
            out.push_str(&text[a.0..a.1]);
            out.push_str(&text[b.1..]);
            out
        }
        3 => replace(text, start, start, &format!("{} ", spelling(rng))),
        4 => replace(text, start, end, spelling(rng)),
        _ => {
            // Another name of the same module, as an agent mixes names up.
            let names: Vec<&str> = spans
                .iter()
                .filter(|(_, _, k)| matches!(k, TokenKind::Ident(_)))
                .map(|(s, e, _)| &text[*s..*e])
                .collect();
            match names.get(rng.below(names.len())) {
                Some(name) => replace(text, start, end, name),
                None => replace(text, start, end, spelling(rng)),
            }
        }
    }
}

/// An integer literal at an edge, or a long digit string, maybe signed and suffixed.
pub fn edge_literal(rng: &mut Rng) -> String {
    let digits = match rng.below(10) {
        0 => {
            let n = 30 + rng.below(400);
            let mut s = String::from("1");
            for _ in 1..n {
                s.push(char::from(b'0' + rng.below(10) as u8));
            }
            s
        }
        1 => format!("{}1", "0".repeat(1 + rng.below(100))),
        _ => rng.pick(&EDGES).to_string(),
    };
    let sign = if rng.one_in(4) { "-" } else { "" };
    let suffix = if rng.one_in(3) {
        rng.pick(&SUFFIXES)
    } else {
        ""
    };
    format!("{}{}{}", sign, digits, suffix)
}

fn literals(rng: &mut Rng, text: &str) -> String {
    let spans = token_spans(text);
    let ints: Vec<(usize, usize)> = spans
        .iter()
        .filter(|(_, _, k)| matches!(k, TokenKind::IntLit(..)))
        .map(|(s, e, _)| (*s, *e))
        .collect();
    let names: Vec<(usize, usize)> = spans
        .iter()
        .filter(|(_, _, k)| matches!(k, TokenKind::Ident(_)))
        .map(|(s, e, _)| (*s, *e))
        .collect();
    let literal = edge_literal(rng);
    if let Some((s, e)) = ints.get(rng.below(ints.len())).filter(|_| !rng.one_in(5)) {
        replace(text, *s, *e, &literal)
    } else if let Some((s, e)) = names.get(rng.below(names.len())) {
        replace(text, *s, *e, &literal)
    } else {
        format!("{}\npub fn edge() -> i128 = {}\n", text, literal)
    }
}

fn lines(rng: &mut Rng, text: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let n = lines.len();
    if n == 0 {
        return text.to_string();
    }
    let i = rng.below(n);
    let mut out: Vec<&str> = lines.clone();
    match rng.below(7) {
        0 => out.insert(rng.below(n + 1), lines[i]),
        1 => {
            let end = (i + 1 + rng.below(8)).min(n);
            out.drain(i..end);
        }
        2 => out.swap(i, rng.below(n)),
        3 => return text[..boundary(rng, text)].to_string(),
        4 | 5 => {
            let starts = mente_syntax::split_items(text);
            let k = rng.below(starts.len());
            let start = starts.get(k).map_or(0, |s| s.offset);
            let end = starts.get(k + 1).map_or(text.len(), |s| s.offset);
            let (start, end) = (floor(text, start), floor(text, end));
            return if rng.one_in(2) {
                replace(text, start, end, "")
            } else {
                replace(text, end, end, &text[start..end])
            };
        }
        _ => {
            let line = out.remove(i);
            out.insert(rng.below(n), line);
        }
    }
    out.concat()
}

/// The top-level items of `text`, as `split_items` divides it.
fn items(text: &str) -> Vec<&str> {
    let starts = mente_syntax::split_items(text);
    let mut out = Vec::new();
    for (k, s) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map_or(text.len(), |n| n.offset);
        if let Some(item) = text.get(s.offset..end) {
            out.push(item);
        }
    }
    out
}

fn splice(rng: &mut Rng, text: &str, others: &[String]) -> String {
    let Some(other) = others.get(rng.below(others.len())) else {
        return lines(rng, text);
    };
    let theirs = items(other);
    // A function most of the time.
    let functions: Vec<&str> = theirs
        .iter()
        .copied()
        .filter(|i| i.contains("fn "))
        .collect();
    let pool = if functions.is_empty() || rng.one_in(4) {
        &theirs
    } else {
        &functions
    };
    let starts = mente_syntax::split_items(text);
    let k = rng.below(starts.len() + 1);
    let at = floor(text, starts.get(k).map_or(text.len(), |s| s.offset));
    match pool.get(rng.below(pool.len())) {
        Some(item) if !rng.one_in(4) => {
            let sep = if text[..at].ends_with('\n') || at == 0 {
                ""
            } else {
                "\n"
            };
            if rng.one_in(3) && k < starts.len() {
                // Replace the item there.
                let end = floor(text, starts.get(k + 1).map_or(text.len(), |s| s.offset));
                replace(text, at, end, &format!("{}{}", sep, item))
            } else {
                replace(text, at, at, &format!("{}{}", sep, item))
            }
        }
        _ => {
            // A block of lines, anywhere.
            let lines: Vec<&str> = other.split_inclusive('\n').collect();
            let i = rng.below(lines.len());
            let end = (i + 1 + rng.below(12)).min(lines.len());
            let block = lines.get(i..end).map(|l| l.concat()).unwrap_or_default();
            let starts: Vec<usize> = std::iter::once(0)
                .chain(text.match_indices('\n').map(|(p, _)| p + 1))
                .collect();
            let at = starts[rng.below(starts.len())];
            replace(text, at, at, &block)
        }
    }
}

/// The way of nesting, of [`nest`]'s ten, that is a statement rather than an expression.
const NEST_STATEMENT: usize = 9;

/// `atom` nested `depth` times, in one of ten ways; the last is a statement, for a block body.
fn nest(kind: usize, depth: usize, atom: &str) -> String {
    match kind {
        0 => format!("{}{}{}", "(".repeat(depth), atom, ")".repeat(depth)),
        1 => format!("{}{}", "- ".repeat(depth), atom),
        2 => format!("{}{}", "not ".repeat(depth), atom),
        3 => format!("{}{{ {} }}", "if x == 0 { 0 } else ".repeat(depth), atom),
        4 => format!(
            "{}{}{}",
            "if true { ".repeat(depth),
            atom,
            " } else { 0 }".repeat(depth)
        ),
        5 => format!("{}{}", atom, " + 1".repeat(depth)),
        6 => format!("{}{}{}", "[".repeat(depth), atom, "]".repeat(depth)),
        7 => format!("{}{}{}", "f(".repeat(depth), atom, ")".repeat(depth)),
        8 => format!("{}{}", atom, "[0]".repeat(depth)),
        _ => format!(
            "{}{}\n  {}",
            "while x > 0 { ".repeat(depth),
            " }".repeat(depth),
            atom
        ),
    }
}

/// A function of its own whose body nests `depth` times in the way `kind` names.
fn deep_function(kind: usize, depth: usize) -> String {
    if kind == NEST_STATEMENT {
        format!(
            "pub fn deep(x: i32) -> i32 {{\n  {}\n}}\n",
            nest(kind, depth, "x")
        )
    } else {
        format!("pub fn deep(x: i32) -> i32 = {}\n", nest(kind, depth, "x"))
    }
}

fn nesting(rng: &mut Rng, text: &str) -> String {
    let depth = [2, 3, 8, 16, 64, 200, 1000, 2000, 5000, 20000, 50000][rng.below(11)];
    let kind = rng.below(10);
    let spans = token_spans(text);
    // An atom of a body: a literal, or a name after `=`, `(` or an operator.
    let atoms: Vec<(usize, usize)> = spans
        .windows(2)
        .filter(|w| {
            matches!(w[1].2, TokenKind::IntLit(..) | TokenKind::Ident(_))
                && matches!(
                    w[0].2,
                    TokenKind::Eq
                        | TokenKind::LParen
                        | TokenKind::Plus
                        | TokenKind::Minus
                        | TokenKind::Star
                        | TokenKind::Lt
                        | TokenKind::Return
                        | TokenKind::LBrace
                )
        })
        .map(|w| (w[1].0, w[1].1))
        .collect();
    match atoms.get(rng.below(atoms.len())) {
        // Half the time in a function of its own, which parses whatever the seed holds.
        Some((s, e)) if kind != NEST_STATEMENT && rng.one_in(2) => {
            replace(text, *s, *e, &nest(kind, depth, &text[*s..*e]))
        }
        _ => format!("{}\n{}", text, deep_function(kind, depth)),
    }
}

/// The edit the incremental re-check applies: one light mutation, the same for one seed.
pub fn edit_text(text: &str, seed: u64) -> String {
    let mut rng = Rng::new(seed);
    match rng.below(4) {
        0 => literals(&mut rng, text),
        1 => lines(&mut rng, text),
        _ => tokens(&mut rng, text),
    }
}

// -------------------------------------------------------------------------------------------
// Failures and the phases
// -------------------------------------------------------------------------------------------

/// One input for every phase: its text, the seed of the edit the incremental re-check makes,
/// and whether the run is thorough: each phase whose output is compared runs twice, and the
/// entry points that only repeat the checker's work under another name, the typed dump and
/// Tier 1's reports, run too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    pub text: String,
    pub edit: u64,
    pub thorough: bool,
}

/// What went wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Panic,
    /// The process ended without a word: a stack overflow, an abort, running out of memory.
    Crash,
    Hang,
    BrokenMir,
    NotLowered,
    NotBuilt,
    /// Two runs of one phase on one input gave different output.
    Unstable,
    /// The incremental re-check reported otherwise than a whole check.
    Disagrees,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Panic => "panic",
            Kind::Crash => "crash",
            Kind::Hang => "hang",
            Kind::BrokenMir => "broken-mir",
            Kind::NotLowered => "lowering-error",
            Kind::NotBuilt => "codegen-error",
            Kind::Unstable => "unstable",
            Kind::Disagrees => "disagrees",
        }
    }

    fn named(name: &str) -> Kind {
        [
            Kind::Panic,
            Kind::Crash,
            Kind::Hang,
            Kind::BrokenMir,
            Kind::NotLowered,
            Kind::NotBuilt,
            Kind::Unstable,
            Kind::Disagrees,
        ]
        .into_iter()
        .find(|k| k.name() == name)
        .unwrap_or(Kind::Crash)
    }
}

/// A failure of the oracle, in the phase it happened in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub phase: String,
    pub kind: Kind,
    pub message: String,
}

impl Failure {
    /// What makes two failures one bug: a panic's place in the compiler, whatever phase met
    /// it; a hang's phase; a disagreement's moment, the first check or a re-check; otherwise
    /// the phase, the kind and the message, with the numbers and names that vary from input to
    /// input left out. A lowering or codegen error is one bug with proofs and without.
    pub fn key(&self) -> String {
        match self.kind {
            Kind::Panic => match self.message.rfind(" [at ") {
                Some(at) => format!("panic{}", &self.message[at..]),
                None => format!("panic in {}: {}", self.phase, normalize(&self.message)),
            },
            Kind::Hang => format!("hang in {}", self.phase),
            Kind::Disagrees => format!(
                "disagrees in {}: {}",
                self.phase,
                self.message.split(':').next().unwrap_or("")
            ),
            kind => format!(
                "{} in {}: {}",
                kind.name(),
                match kind {
                    Kind::BrokenMir => &self.phase,
                    _ => self.phase.trim_end_matches("-without-proofs"),
                },
                normalize(&self.message)
            ),
        }
    }
}

/// `message` without what varies between inputs that meet one bug: numbers, names in
/// backticks, and the function a lowering error names.
pub fn normalize(message: &str) -> String {
    let mut rest = message;
    while let Some(after) = rest
        .strip_prefix("fn ")
        .or_else(|| rest.strip_prefix("entry of "))
    {
        match after.find(": ") {
            Some(colon) => rest = &after[colon + 2..],
            None => break,
        }
    }
    let mut out = String::new();
    let mut quoted = false;
    for c in rest.chars() {
        if c == '`' {
            quoted = !quoted;
            if quoted {
                out.push_str("`_`");
            }
            continue;
        }
        if quoted {
            continue;
        }
        if !c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('#') {
            out.push('#');
        }
    }
    out
}

/// What a phase run reports as it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Phase(String),
    Failure(Failure),
}

/// What one input did: the phases it reached, in order, and its failures.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub reached: Vec<String>,
    pub failures: Vec<Failure>,
    /// The time spent in each phase reached, as a worker measures it.
    pub times: Vec<Duration>,
}

impl Outcome {
    #[cfg(test)]
    fn of(events: Vec<Event>) -> Outcome {
        let mut out = Outcome::default();
        for e in events {
            match e {
                Event::Phase(p) => out.reached.push(p),
                Event::Failure(f) => out.failures.push(f),
            }
        }
        out
    }

    pub fn reached(&self, phase: &str) -> bool {
        self.reached.iter().any(|p| p == phase)
    }
}

/// The compiler's phases, and how they run.
#[derive(Clone, Copy, Debug)]
pub struct Phases {
    /// How long one phase may take on one input.
    pub budget: Duration,
    /// The stack of the thread the phases run on.
    pub stack: usize,
    /// Whether a module that checks gets an object for each target.
    pub objects: bool,
    /// A phase of the fuzzer's own tests, run right after the lexer.
    pub plant: Option<fn(&str)>,
}

impl Default for Phases {
    fn default() -> Phases {
        Phases {
            budget: Duration::from_secs(PHASE_SECONDS),
            stack: STACK_BYTES,
            objects: true,
            plant: None,
        }
    }
}

/// One phase run after another, each reported before it starts, its panic caught and its time
/// measured.
struct Run<'a> {
    budget: Duration,
    thorough: bool,
    sink: &'a mut (dyn FnMut(Event) + Send),
}

impl Run<'_> {
    fn fail(&mut self, phase: &str, kind: Kind, message: String) {
        (self.sink)(Event::Failure(Failure {
            phase: phase.to_string(),
            kind,
            message,
        }));
    }

    /// Runs `f` as the phase `name`; `None` when it panicked.
    fn phase<T>(&mut self, name: &str, f: impl FnOnce() -> T) -> Option<T> {
        (self.sink)(Event::Phase(name.to_string()));
        let start = Instant::now();
        let result = std::panic::catch_unwind(AssertUnwindSafe(f));
        let took = start.elapsed();
        if took > self.budget {
            self.fail(
                name,
                Kind::Hang,
                format!(
                    "took {} ms, over the phase budget of {} ms",
                    took.as_millis(),
                    self.budget.as_millis()
                ),
            );
        }
        match result {
            Ok(v) => Some(v),
            Err(payload) => {
                let message = panic_message(payload.as_ref());
                self.fail(name, Kind::Panic, message);
                None
            }
        }
    }

    /// Runs `f` as the phase `name`, twice when the run is thorough, and reports two
    /// different results.
    fn stable<T: PartialEq>(&mut self, name: &str, f: impl Fn() -> T) -> Option<T> {
        let first = self.phase(name, &f)?;
        if self.thorough {
            let second = self.phase(name, &f)?;
            if second != first {
                self.fail(
                    name,
                    Kind::Unstable,
                    "two runs on one input gave different output".to_string(),
                );
            }
        }
        Some(first)
    }

    /// Reports `message` as a failure of `kind` in `name`, if there is one.
    fn report(&mut self, name: &str, kind: Kind, message: Option<String>) {
        if let Some(m) = message {
            self.fail(name, kind, m);
        }
    }
}

impl Phases {
    /// Runs every phase `case` reaches on a thread of its own, reporting each event to `sink`
    /// as it happens.
    pub fn run(&self, case: &Case, sink: &mut (dyn FnMut(Event) + Send)) {
        install_hook();
        let not_started = std::thread::scope(|s| {
            let spawned = std::thread::Builder::new()
                .name(PHASE_THREAD.to_string())
                .stack_size(self.stack)
                .spawn_scoped(s, || {
                    let mut run = Run {
                        budget: self.budget,
                        thorough: case.thorough,
                        sink: &mut *sink,
                    };
                    self.all(&case.text, case.edit, &mut run);
                });
            match spawned {
                Ok(handle) => {
                    // Every phase catches its own panic, so the thread ends normally.
                    let _ = handle.join();
                    None
                }
                Err(e) => Some(e.to_string()),
            }
        });
        if let Some(e) = not_started {
            sink(Event::Failure(Failure {
                phase: "start".into(),
                kind: Kind::Crash,
                message: format!("the phase thread could not start: {}", e),
            }));
        }
    }

    fn all(&self, src: &str, edit: u64, run: &mut Run) {
        run.phase("lex", || mente_syntax::lex(src));
        if let Some(plant) = self.plant {
            run.phase("planted", || plant(src));
        }
        run.stable("dump-tokens", || mente_syntax::dump_tokens(src));
        run.phase("split-items", || mente_syntax::split_items(src));
        let parsed = run
            .phase("parse", || mente_syntax::parse_module(src).ok())
            .flatten();
        if let Some(module) = &parsed {
            run.phase("parse-bodies", || mente_syntax::parse_bodies(src, module));
            run.phase("blueprint", || mente_ast::project_spec_blueprint(module));
        }
        run.stable("dump-tree", || mente_syntax::dump_tree(src));
        let diagnostics = run.stable("check", || diagnostics_of(src));
        if let Some(diagnostics) = &diagnostics {
            run.phase("render", || render(src, diagnostics));
            let disagreement = run
                .phase("incremental", || incremental(src, edit, diagnostics))
                .flatten();
            run.report("incremental", Kind::Disagrees, disagreement);
        }
        if run.thorough {
            run.stable("dump-typed", || mente_sema::dump_typed(src));
            run.phase("tier1", || mente_sema::tier1_reports(src).len());
        }
        let proven = run
            .phase("check-program", || mente_sema::check_program(src).ok())
            .flatten();
        let unproven = run
            .phase("check-without-proofs", || {
                mente_sema::check_program_without_proofs(src).ok()
            })
            .flatten();
        let Some(module) = proven else {
            return;
        };
        let functions = scalar_functions(&module);
        let lowered = self.lower(run, "lower", &module);
        if let Some(lowered) = &lowered {
            run.stable("dump-mir", || lowered.module.to_string());
            if self.objects {
                self.objects(run, lowered, &functions);
            }
        }
        let panicked = run
            .phase("interp", || interpret(&module, &functions))
            .flatten();
        run.report("interp", Kind::Panic, panicked);
        if let Some(lowered) = &lowered {
            let broken = run.phase("mir", || run_mir(lowered, &functions)).flatten();
            run.report("mir", Kind::BrokenMir, broken);
        }
        if let Some(unproven) = unproven {
            if let Some(lowered) = self.lower(run, "lower-without-proofs", &unproven) {
                let broken = run
                    .phase("mir-without-proofs", || run_mir(&lowered, &functions))
                    .flatten();
                run.report("mir-without-proofs", Kind::BrokenMir, broken);
            }
        }
    }

    /// Lowers a module that checks; an error is a failure.
    fn lower(&self, run: &mut Run, name: &str, module: &CheckedModule) -> Option<Lowered> {
        let lower = || mente_codegen::lower::lower(module);
        match run.phase(name, lower)? {
            Err(e) => {
                run.fail(name, Kind::NotLowered, e);
                None
            }
            Ok(lowered) => {
                if run.thorough {
                    let again = run.phase(name, lower)?;
                    if again.map(|l| l.module.to_string()) != Ok(lowered.module.to_string()) {
                        run.fail(
                            name,
                            Kind::Unstable,
                            "two runs on one input gave different output".to_string(),
                        );
                    }
                }
                Some(lowered)
            }
        }
    }

    /// Writes the object of each target, without linking; an error is a failure.
    fn objects(&self, run: &mut Run, lowered: &Lowered, functions: &[Function]) {
        let calls: Vec<native::Call> = functions
            .iter()
            .filter_map(|f| {
                let entry = lowered.entries.iter().position(|e| e.name == f.name)?;
                let args = f.params.iter().map(|p| arg(&Scalar::zero(p))).collect();
                Some(native::Call { entry, args })
            })
            .collect();
        for (k, target) in mente_codegen::Target::ALL.into_iter().enumerate() {
            let name = format!("object {}", target.triple());
            let build = || {
                native::object(
                    lowered,
                    &calls,
                    "fuzz.mn: ",
                    mente_codegen::SourceFile {
                        path: "fuzz.mn".into(),
                        directory: ".".into(),
                    },
                    "fuzz.o",
                    target,
                )
                .map_err(|e| e.to_string())
            };
            // Objects are compared on the first target only, to keep runs short.
            let built = if k == 0 {
                run.stable(&name, build)
            } else {
                run.phase(&name, build)
            };
            if let Some(Err(e)) = built {
                run.fail(&name, Kind::NotBuilt, e);
            }
        }
    }
}

/// What `check_source` reports, nothing for a module that checks.
fn diagnostics_of(src: &str) -> Vec<Diagnostic> {
    mente_sema::check_source(src).err().unwrap_or_default()
}

/// Renders each diagnostic in the text form and all of them in the JSON form, as `mn check`
/// prints them.
fn render(src: &str, diagnostics: &[Diagnostic]) -> usize {
    let mut size = 0;
    for d in diagnostics {
        size += render_text(d, "fuzz.mn", src).len();
    }
    let listings = mente_sema::listings(src);
    size + render_json(&[FileDiagnostics {
        file: "fuzz.mn",
        src,
        diagnostics,
        todos: &listings.todos,
        axioms: &listings.axioms,
    }])
    .len()
}

/// Checks `src` item by item, as `mn check --watch` does, then again after an edit, and says
/// how it differs from a whole check, if it does.
fn incremental(src: &str, edit: u64, whole: &[Diagnostic]) -> Option<String> {
    let mut session = mente_sema::incremental::Session::new(src);
    if session.diagnostics() != whole {
        return Some(format!(
            "{}: the session reports {}, a whole check {}",
            FIRST_CHECK,
            codes(session.diagnostics()),
            codes(whole)
        ));
    }
    let edited = edit_text(src, edit);
    session.update(&edited);
    let fresh = diagnostics_of(&edited);
    (session.diagnostics() != fresh.as_slice()).then(|| {
        format!(
            "{}: the session reports {}, a whole check {}",
            RE_CHECK,
            codes(session.diagnostics()),
            codes(&fresh)
        )
    })
}

/// How a disagreement of the incremental check starts: on a module's first check, or on the
/// re-check after an edit.
const FIRST_CHECK: &str = "the first check differs from a whole check";
const RE_CHECK: &str = "the re-check after an edit differs from a whole check";

/// A case that shows `failure` on its own, when there is one: a re-check that disagrees after
/// an edit, whose edited text disagrees on its first check too, is that text's bug.
fn standalone(
    exec: &mut dyn Executor,
    case: &Case,
    failure: Failure,
) -> Result<(Case, Failure), String> {
    if failure.kind != Kind::Disagrees || !failure.message.starts_with(RE_CHECK) {
        return Ok((case.clone(), failure));
    }
    let edited = Case {
        text: edit_text(&case.text, case.edit),
        ..case.clone()
    };
    let first = exec
        .exec(&edited)?
        .failures
        .into_iter()
        .find(|f| f.kind == Kind::Disagrees && f.message.starts_with(FIRST_CHECK));
    Ok(match first {
        Some(f) => (edited, f),
        None => (case.clone(), failure),
    })
}

fn codes(diagnostics: &[Diagnostic]) -> String {
    let codes: Vec<String> = diagnostics
        .iter()
        .map(|d| format!("{} at {}:{}", d.code, d.primary.line, d.primary.col))
        .collect();
    format!("[{}]", codes.join(", "))
}

// -------------------------------------------------------------------------------------------
// Running functions on edge values
// -------------------------------------------------------------------------------------------

/// A scalar argument: an integer by sign and magnitude, or a `bool`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scalar {
    Int(bool, u128),
    Bool(bool),
}

impl Scalar {
    fn zero(shape: &Shape) -> Scalar {
        match shape {
            Shape::Bool => Scalar::Bool(false),
            _ => Scalar::Int(false, 0),
        }
    }
}

/// The values worth calling a parameter of `shape` with: 0, 1, its maximum, and for a signed
/// type -1 and its minimum; `false` and `true`.
fn edges(shape: &Shape) -> Vec<Scalar> {
    match shape {
        Shape::Int(t) if t.signed => {
            let max = (1u128 << (t.bits - 1)) - 1;
            vec![
                Scalar::Int(false, 0),
                Scalar::Int(false, max),
                Scalar::Int(true, max + 1),
                Scalar::Int(true, 1),
                Scalar::Int(false, 1),
            ]
        }
        Shape::Int(t) => {
            let max = if t.bits >= 128 {
                u128::MAX
            } else {
                (1u128 << t.bits) - 1
            };
            vec![
                Scalar::Int(false, 0),
                Scalar::Int(false, max),
                Scalar::Int(false, 1),
            ]
        }
        _ => vec![Scalar::Bool(false), Scalar::Bool(true)],
    }
}

/// The calls of a function: every parameter at its first edge, then at its second and its
/// third, then edges mixed.
fn edge_calls(params: &[Shape]) -> Vec<Vec<Scalar>> {
    let edges: Vec<Vec<Scalar>> = params.iter().map(edges).collect();
    (0..4)
        .map(|k| {
            edges
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let at = if k < 3 { k } else { i + 1 };
                    e[at % e.len()]
                })
                .collect()
        })
        .collect()
}

/// A function whose parameters are all integers or `bool`s.
struct Function {
    name: String,
    params: Vec<Shape>,
}

fn scalar_functions(module: &CheckedModule) -> Vec<Function> {
    module
        .functions
        .iter()
        .flatten()
        .filter_map(|f| {
            let tree = &f.syntax.tree;
            let params = f
                .syntax
                .params
                .iter()
                .map(|p| Shape::of(&mente_sema::types::Ty::of(tree, p.ty)).ok())
                .collect::<Option<Vec<Shape>>>()?;
            params
                .iter()
                .all(|p| matches!(p, Shape::Int(_) | Shape::Bool))
                .then(|| Function {
                    name: f.syntax.name.clone(),
                    params,
                })
        })
        .take(MAX_FUNCTIONS)
        .collect()
}

fn value(s: &Scalar) -> Value {
    match *s {
        Scalar::Bool(b) => Value::Bool(b),
        Scalar::Int(neg, mag) if neg => Value::Int(Int::from_u128(mag).negate()),
        Scalar::Int(_, mag) => Value::Int(Int::from_u128(mag)),
    }
}

fn arg(s: &Scalar) -> native::Arg {
    match *s {
        Scalar::Bool(b) => native::Arg::Bool(b),
        Scalar::Int(neg, mag) => native::Arg::Int(Num::new(neg, mag)),
    }
}

/// Runs every edge call on the reference interpreter, and says how it panicked, if it did.
fn interpret(module: &CheckedModule, functions: &[Function]) -> Option<String> {
    let limits = mente_interp::Limits {
        max_depth: MAX_DEPTH,
        max_steps: REFERENCE_STEPS,
        max_array: REFERENCE_ARRAY,
    };
    for f in functions {
        for call in edge_calls(&f.params) {
            let args = call.iter().map(value).collect();
            if let Err(mente_interp::Stop::BadArguments(why)) =
                mente_interp::call(module, &f.name, args, limits)
            {
                // The interpreter runs on a thread of its own, and reports its panic this way.
                if why.contains("stopped unexpectedly") {
                    return Some(format!(
                        "the reference interpreter panicked: {}",
                        last_panic()
                    ));
                }
            }
        }
    }
    None
}

/// Runs every edge call on MIR, and says how the MIR broke, if it did.
fn run_mir(lowered: &Lowered, functions: &[Function]) -> Option<String> {
    let limits = cfg::Limits {
        max_depth: MAX_DEPTH,
        max_steps: MAX_STEPS,
    };
    for f in functions {
        let Some(entry) = lowered.entries.iter().find(|e| e.name == f.name) else {
            continue;
        };
        'calls: for call in edge_calls(&f.params) {
            let args: Vec<native::Arg> = call.iter().map(arg).collect();
            if native::refusal(entry, &args).is_some() || call.len() != entry.params.len() {
                continue;
            }
            let mut memory = Memory::default();
            let mut values = Vec::new();
            if let Shape::Array(..) = entry.result {
                values.push(Val::Addr(memory.alloc(entry.result.size())));
            }
            for ((is_mut, shape), s) in entry.params.iter().zip(&call) {
                let v = match *s {
                    Scalar::Bool(b) => Val::Bool(b),
                    Scalar::Int(neg, mag) => Val::Int(Num::new(neg, mag)),
                };
                if *is_mut {
                    let at = memory.alloc(shape.size());
                    if memory.store(at, v, shape.ty()).is_err() {
                        continue 'calls;
                    }
                    values.push(Val::Addr(at));
                } else {
                    values.push(v);
                }
            }
            if let Err(cfg::Stop::Broken(why)) =
                cfg::run(&lowered.module, entry.func, &values, &mut memory, limits)
            {
                return Some(format!("broken MIR: {}", why));
            }
        }
    }
    None
}

// -------------------------------------------------------------------------------------------
// Panics
// -------------------------------------------------------------------------------------------

/// The name of the thread the phases run on, whose panics the hook keeps quiet.
const PHASE_THREAD: &str = "mn-fuzz-phases";

/// Whether the hook keeps every panic quiet, as in a worker, whose standard error nobody reads.
static SILENT: AtomicBool = AtomicBool::new(false);

/// The last panic on any thread, and the last on each phase thread, with its place.
static LAST_PANIC: Mutex<String> = Mutex::new(String::new());
static PHASE_PANICS: Mutex<Vec<(std::thread::ThreadId, String)>> = Mutex::new(Vec::new());
static HOOK: Once = Once::new();

/// Installs, once, a panic hook that records each panic's message and place, and passes the
/// panic on to the hook before it unless it happened in a phase thread.
fn install_hook() {
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let payload = info.payload();
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a panic without a message".to_string());
            let text = match info.location() {
                Some(l) => format!("{} [at {}:{}:{}]", message, l.file(), l.line(), l.column()),
                None => message,
            };
            let current = std::thread::current();
            let quiet = current.name() == Some(PHASE_THREAD);
            if let Ok(mut last) = LAST_PANIC.lock() {
                last.clone_from(&text);
            }
            if quiet {
                if let Ok(mut all) = PHASE_PANICS.lock() {
                    all.retain(|(id, _)| *id != current.id());
                    all.push((current.id(), text));
                }
            }
            if !quiet && !SILENT.load(Ordering::Relaxed) {
                previous(info);
            }
        }));
    });
}

/// The message and place of the panic this phase thread just caught.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    let id = std::thread::current().id();
    let recorded = PHASE_PANICS
        .lock()
        .ok()
        .and_then(|all| all.iter().find(|(i, _)| *i == id).map(|(_, t)| t.clone()));
    recorded.unwrap_or_else(|| {
        payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic without a message".to_string())
    })
}

fn last_panic() -> String {
    LAST_PANIC
        .lock()
        .map(|l| l.clone())
        .unwrap_or_else(|_| "a panic whose message was lost".to_string())
}

/// The planted failures `MENTE_FUZZ_PLANT` names.
fn plant_from_env() -> Option<fn(&str)> {
    match std::env::var("MENTE_FUZZ_PLANT").ok()?.as_str() {
        "panic" => Some(plant_panic),
        "overflow" => Some(plant_overflow),
        "hang" => Some(plant_hang),
        _ => None,
    }
}

fn plant_panic(src: &str) {
    if src.contains(PLANT_MARK) {
        panic!("a planted panic");
    }
}

fn plant_overflow(src: &str) {
    if src.contains(PLANT_MARK) {
        std::hint::black_box(descend(0));
    }
}

/// A recursion without a limit.
fn descend(depth: u64) -> u64 {
    let frame = std::hint::black_box([depth; 64]);
    if depth == u64::MAX {
        return frame[0];
    }
    descend(depth + 1).wrapping_add(frame[1])
}

fn plant_hang(src: &str) {
    if src.contains(PLANT_MARK) {
        loop {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

// -------------------------------------------------------------------------------------------
// Executors: in this process, or in a worker process
// -------------------------------------------------------------------------------------------

/// Runs one case through every phase and returns what it did; an error means the fuzzer
/// itself cannot go on.
pub trait Executor {
    fn exec(&mut self, case: &Case) -> Result<Outcome, String>;
}

/// The phases in this process: for tests, whose inputs never nest deeply.
#[cfg(test)]
pub struct InProcess(pub Phases);

#[cfg(test)]
impl Executor for InProcess {
    fn exec(&mut self, case: &Case) -> Result<Outcome, String> {
        let mut events = Vec::new();
        self.0.run(case, &mut |e| events.push(e));
        Ok(Outcome::of(events))
    }
}

/// The phases in a worker process, started again whenever it dies or stops answering.
pub struct Worker {
    exe: PathBuf,
    args: Vec<String>,
    budget: Duration,
    live: Option<Live>,
    pub restarts: usize,
}

struct Live {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
    stderr: Arc<Mutex<String>>,
    stderr_reader: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    pub fn new(exe: PathBuf, phases: &Phases) -> Worker {
        Worker {
            exe,
            args: vec![
                "fuzz".into(),
                "--worker".into(),
                "--phase-seconds".into(),
                phases.budget.as_secs().max(1).to_string(),
                "--stack-kib".into(),
                (phases.stack >> 10).to_string(),
            ],
            budget: phases.budget,
            live: None,
            restarts: 0,
        }
    }

    fn start(&mut self) -> Result<&mut Live, String> {
        if self.live.is_none() {
            let mut child = Command::new(&self.exe)
                .args(&self.args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("cannot start the worker {}: {}", self.exe.display(), e))?;
            let (Some(stdin), Some(stdout), Some(mut stderr)) =
                (child.stdin.take(), child.stdout.take(), child.stderr.take())
            else {
                return Err("the worker has no pipes".into());
            };
            let (tx, lines) = mpsc::channel();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            });
            let tail = Arc::new(Mutex::new(String::new()));
            let keep = Arc::clone(&tail);
            let stderr_reader = std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = stderr.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut t) = keep.lock() {
                        t.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if t.len() > 16 << 10 {
                            let cut = floor(t.as_str(), t.len() - (8 << 10));
                            t.replace_range(..cut, "");
                        }
                    }
                }
            });
            self.restarts += 1;
            self.live = Some(Live {
                child,
                stdin,
                lines,
                stderr: tail,
                stderr_reader: Some(stderr_reader),
            });
        }
        self.live.as_mut().ok_or_else(|| "no worker".to_string())
    }

    /// Stops the worker and returns how it ended and the end of what it wrote to standard
    /// error.
    fn stop(&mut self) -> (String, String) {
        let Some(mut live) = self.live.take() else {
            return (String::new(), String::new());
        };
        let _ = live.child.kill();
        let status = live
            .child
            .wait()
            .map(|s| s.to_string())
            .unwrap_or_else(|e| e.to_string());
        if let Some(reader) = live.stderr_reader.take() {
            let _ = reader.join();
        }
        let tail = live.stderr.lock().map(|t| t.clone()).unwrap_or_default();
        (status, tail)
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Executor for Worker {
    fn exec(&mut self, case: &Case) -> Result<Outcome, String> {
        let budget = self.budget;
        let mut sent = false;
        for _ in 0..2 {
            let live = self.start()?;
            if let Ok(mut t) = live.stderr.lock() {
                t.clear();
            }
            let header = format!(
                "case {} {} {}\n",
                case.text.len(),
                case.edit,
                u8::from(case.thorough)
            );
            let wrote = live
                .stdin
                .write_all(header.as_bytes())
                .and_then(|()| live.stdin.write_all(case.text.as_bytes()))
                .and_then(|()| live.stdin.flush());
            if wrote.is_ok() {
                sent = true;
                break;
            }
            // The worker died after its last answer: start another.
            self.stop();
        }
        if !sent {
            return Err("the worker does not take input".into());
        }
        let mut outcome = Outcome::default();
        let mut last = "start".to_string();
        let mut since = Instant::now();
        let mut lap = |outcome: &mut Outcome| {
            if outcome.times.len() < outcome.reached.len() {
                outcome.times.push(since.elapsed());
            }
            since = Instant::now();
        };
        loop {
            let Some(live) = self.live.as_mut() else {
                return Err("the worker vanished".into());
            };
            match live.lines.recv_timeout(budget + GRACE) {
                Ok(line) => {
                    if let Some(phase) = line.strip_prefix("phase ") {
                        lap(&mut outcome);
                        last = phase.to_string();
                        outcome.reached.push(last.clone());
                    } else if let Some(rest) = line.strip_prefix("fail ") {
                        let mut parts = rest.splitn(3, '\t');
                        let phase = parts.next().unwrap_or("").to_string();
                        let kind = Kind::named(parts.next().unwrap_or(""));
                        let message = unescape(parts.next().unwrap_or(""));
                        outcome.failures.push(Failure {
                            phase,
                            kind,
                            message,
                        });
                    } else if line == "done" {
                        lap(&mut outcome);
                        return Ok(outcome);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    lap(&mut outcome);
                    self.stop();
                    outcome.failures.push(Failure {
                        phase: last,
                        kind: Kind::Hang,
                        message: format!(
                            "no answer within {} s, the phase budget and a grace",
                            (budget + GRACE).as_secs()
                        ),
                    });
                    return Ok(outcome);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    lap(&mut outcome);
                    let (status, tail) = self.stop();
                    outcome.failures.push(Failure {
                        phase: last,
                        kind: Kind::Crash,
                        message: crash_message(&status, &tail),
                    });
                    return Ok(outcome);
                }
            }
        }
    }
}

/// Names how a worker ended: a stack overflow, running out of memory, or its status and the
/// last line it wrote.
fn crash_message(status: &str, stderr: &str) -> String {
    if stderr.contains("has overflowed its stack") || status.contains("0xc00000fd") {
        "stack overflow".to_string()
    } else if stderr.contains("memory allocation of") {
        "out of memory".to_string()
    } else {
        let last = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        format!("the worker ended with {}: {}", status, last)
    }
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// The worker's loop: reads each case from standard input, runs every phase it reaches and
/// writes each event on a line of standard output, then `done`.
fn worker(phases: Phases) -> i32 {
    SILENT.store(true, Ordering::Relaxed);
    install_hook();
    let mut input = BufReader::new(std::io::stdin().lock());
    let stdout = std::io::stdout();
    loop {
        let mut header = String::new();
        match input.read_line(&mut header) {
            Ok(0) | Err(_) => return 0,
            Ok(_) => {}
        }
        let fields: Vec<&str> = header.split_whitespace().collect();
        let (Some(len), Some(edit), Some(thorough)) = (
            fields.get(1).and_then(|n| n.parse::<usize>().ok()),
            fields.get(2).and_then(|n| n.parse::<u64>().ok()),
            fields.get(3),
        ) else {
            eprintln!("error: a case header reads `{}`", header.trim_end());
            return 2;
        };
        let mut bytes = vec![0u8; len];
        if input.read_exact(&mut bytes).is_err() {
            return 2;
        }
        let case = Case {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            edit,
            thorough: *thorough == "1",
        };
        phases.run(&case, &mut |event| {
            let line = match event {
                Event::Phase(p) => format!("phase {}\n", p),
                Event::Failure(f) => format!(
                    "fail {}\t{}\t{}\n",
                    f.phase,
                    f.kind.name(),
                    escape(&f.message)
                ),
            };
            let mut out = stdout.lock();
            let _ = out.write_all(line.as_bytes());
            let _ = out.flush();
        });
        let mut out = stdout.lock();
        if out.write_all(b"done\n").and_then(|()| out.flush()).is_err() {
            return 0;
        }
    }
}

// -------------------------------------------------------------------------------------------
// Minimizing
// -------------------------------------------------------------------------------------------

/// The smallest text the minimizer finds that `fails` still holds for: delta debugging over
/// lines, then tokens, then characters, repeated while it shrinks.
pub fn minimize(input: &str, fails: &mut dyn FnMut(&str) -> bool) -> String {
    let mut current = input.to_string();
    for _ in 0..3 {
        let before = current.len();
        for split in [split_lines, split_tokens, split_chars] {
            let units = split(&current);
            current = ddmin(units, fails).concat();
        }
        if current.len() == before {
            break;
        }
    }
    current
}

fn split_lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(String::from).collect()
}

/// Runs of letters, digits and `_`, runs of white space, and every other character alone.
fn split_tokens(text: &str) -> Vec<String> {
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        }
    };
    let mut out: Vec<String> = Vec::new();
    let mut last = None;
    for c in text.chars() {
        let k = class(c);
        match out.last_mut() {
            Some(unit) if last == Some(k) && k != 2 => unit.push(c),
            _ => out.push(c.to_string()),
        }
        last = Some(k);
    }
    out
}

fn split_chars(text: &str) -> Vec<String> {
    text.chars().map(String::from).collect()
}

/// Zeller's delta debugging, removing ever smaller chunks while the failure holds.
fn ddmin(mut units: Vec<String>, fails: &mut dyn FnMut(&str) -> bool) -> Vec<String> {
    let mut n = 2;
    while units.len() >= 2 {
        let chunk = units.len().div_ceil(n);
        let mut reduced = false;
        let mut start = 0;
        while start < units.len() {
            let end = (start + chunk).min(units.len());
            let candidate: Vec<String> = units[..start]
                .iter()
                .chain(&units[end..])
                .cloned()
                .collect();
            if fails(&candidate.concat()) {
                units = candidate;
                n = (n - 1).max(2);
                reduced = true;
                break;
            }
            start = end;
        }
        if !reduced {
            if n >= units.len() {
                break;
            }
            n = (n * 2).min(units.len());
        }
    }
    units
}

/// Minimizes `case`, keeping the failure whose key is `key`, within the minimizer's budget.
fn shrink(exec: &mut dyn Executor, case: &Case, key: &str, deadline: Instant) -> String {
    let start = Instant::now();
    let time = MINIMIZE_TIME.min(MINIMIZE_LATE.max(deadline.saturating_duration_since(start)));
    let mut tries = 0;
    minimize(&case.text, &mut |text| {
        tries += 1;
        if tries > MINIMIZE_TRIES || start.elapsed() > time {
            return false;
        }
        let candidate = Case {
            text: text.to_string(),
            ..case.clone()
        };
        exec.exec(&candidate)
            .map(|o| o.failures.iter().any(|f| f.key() == key))
            .unwrap_or(false)
    })
}

// -------------------------------------------------------------------------------------------
// The campaign
// -------------------------------------------------------------------------------------------

/// The inputs of one job: first its share of the seeds as they are, then mutants of the seeds
/// and of the mutants that checked.
pub struct Generator {
    rng: Rng,
    seeds: VecDeque<String>,
    corpus: Vec<String>,
    families: Vec<Family>,
}

impl Generator {
    /// The inputs of job `job` of `jobs`; with `shallow`, no deep nesting, for phases that run
    /// in the fuzzer's own process.
    pub fn new(seed: u64, job: usize, jobs: usize, corpus: &[String], shallow: bool) -> Generator {
        Generator {
            rng: Rng::new(seed ^ (job as u64).wrapping_mul(0xD1B5_4A32_D192_ED03)),
            seeds: corpus
                .iter()
                .enumerate()
                .filter(|(i, _)| i % jobs.max(1) == job)
                .map(|(_, s)| s.clone())
                .collect(),
            corpus: corpus.to_vec(),
            families: Family::ALL
                .into_iter()
                .filter(|f| !shallow || *f != Family::Nesting)
                .collect(),
        }
    }

    pub fn next_case(&mut self) -> Case {
        if let Some(text) = self.seeds.pop_front() {
            return Case {
                text,
                edit: self.rng.next_u64(),
                thorough: true,
            };
        }
        let base = self
            .corpus
            .get(self.rng.below(self.corpus.len()))
            .cloned()
            .unwrap_or_default();
        let text = mutate(&mut self.rng, &base, &self.corpus, &self.families);
        Case {
            text,
            edit: self.rng.next_u64(),
            thorough: self.rng.one_in(4),
        }
    }

    /// A mutant that checked joins the corpus, while there is room.
    pub fn keep(&mut self, text: String) {
        if self.corpus.len() < MAX_CORPUS {
            self.corpus.push(text);
        }
    }
}

/// A failure found, minimized, and where.
#[derive(Clone, Debug)]
pub struct Found {
    pub failure: Failure,
    pub minimized: String,
    pub original: String,
    pub job: usize,
    pub run: u64,
    pub edit: u64,
}

/// What `mn fuzz` was asked to do.
#[derive(Clone, Debug)]
pub struct Options {
    pub seed: u64,
    pub runs: Option<u64>,
    pub seconds: u64,
    pub jobs: usize,
    pub out: PathBuf,
    pub phases: Phases,
    /// No deep nesting, for phases that run in the fuzzer's own process.
    pub shallow: bool,
}

/// What one job did.
#[derive(Clone, Debug, Default)]
pub struct Tally {
    pub runs: u64,
    /// For each phase, the runs that reached it and the time they spent in it.
    pub reached: BTreeMap<String, (u64, Duration)>,
}

/// The failures found so far, by key; `None` while one is being minimized.
pub type Known = Mutex<BTreeMap<String, Option<Found>>>;

/// Runs one job until its runs or the deadline are spent.
pub fn campaign(
    job: usize,
    opts: &Options,
    corpus: &[String],
    exec: &mut dyn Executor,
    known: &Known,
    deadline: Instant,
) -> Result<Tally, String> {
    let mut generator = Generator::new(opts.seed, job, opts.jobs, corpus, opts.shallow);
    let mut tally = Tally::default();
    let runs = opts.runs.map(|r| r.div_ceil(opts.jobs.max(1) as u64));
    while runs.is_none_or(|r| tally.runs < r) && (opts.runs.is_some() || Instant::now() < deadline)
    {
        let case = generator.next_case();
        let outcome = exec.exec(&case)?;
        let run = tally.runs;
        tally.runs += 1;
        for (k, p) in outcome.reached.iter().enumerate() {
            let entry = tally.reached.entry(p.clone()).or_default();
            entry.0 += 1;
            entry.1 += outcome.times.get(k).copied().unwrap_or_default();
        }
        if outcome.failures.is_empty() && outcome.reached("lower") && run >= corpus.len() as u64 {
            generator.keep(case.text.clone());
        }
        for failure in outcome.failures {
            let (case, failure) = standalone(exec, &case, failure)?;
            let key = failure.key();
            let fresh = known
                .lock()
                .map(|mut k| {
                    let fresh = !k.contains_key(&key);
                    if fresh {
                        k.insert(key.clone(), None);
                    }
                    fresh
                })
                .unwrap_or(false);
            if !fresh {
                continue;
            }
            eprintln!("[fuzz] job {} run {}: new failure: {}", job, run, key);
            let minimized = shrink(exec, &case, &key, deadline);
            let found = Found {
                failure,
                minimized,
                original: case.text.clone(),
                job,
                run,
                edit: case.edit,
            };
            write_found(&opts.out, opts, &found)?;
            if let Ok(mut k) = known.lock() {
                k.insert(key, Some(found));
            }
        }
    }
    Ok(tally)
}

/// A short, stable name for a failure: its phase and kind, and a hash of its key.
fn slug(failure: &Failure) -> String {
    let mut hash: u32 = 0x811C_9DC5;
    for b in failure.key().bytes() {
        hash = (hash ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    let phase: String = failure
        .phase
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("{}-{}-{:08x}", phase, failure.kind.name(), hash)
}

/// Writes a failure's minimized input, its description and its original input to `out`.
fn write_found(out: &Path, opts: &Options, found: &Found) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {}", out.display(), e))?;
    let name = slug(&found.failure);
    let f = &found.failure;
    let about = format!(
        "phase: {}\nkind: {}\nmessage: {}\nkey: {}\nfound by: mn fuzz --seed {} --jobs {}, job {}, run {}\nedit seed: {}\ninput: {} bytes, minimized to {} bytes\nreplay: mn fuzz --replay {}.mn\n",
        f.phase,
        f.kind.name(),
        f.message,
        f.key(),
        opts.seed,
        opts.jobs,
        found.job,
        found.run,
        found.edit,
        found.original.len(),
        found.minimized.len(),
        name
    );
    let write = |file: String, text: &str| {
        let path = out.join(file);
        std::fs::write(&path, text).map_err(|e| format!("{}: {}", path.display(), e))
    };
    write(format!("{}.mn", name), &found.minimized)?;
    write(format!("{}.txt", name), &about)?;
    if found.original.len() <= MAX_NESTED {
        write(format!("{}.orig.mn", name), &found.original)?;
    }
    Ok(())
}

/// Every `.mn` file under `paths`, in a fixed order, that is UTF-8.
pub fn seed_corpus(paths: &[PathBuf]) -> Vec<(PathBuf, String)> {
    fn walk(path: &Path, out: &mut Vec<PathBuf>) {
        if path.is_dir() {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
                .map(|d| d.flatten().map(|e| e.path()).collect())
                .unwrap_or_default();
            entries.sort();
            for e in entries {
                walk(&e, out);
            }
        } else if path.extension().is_some_and(|x| x == "mn") {
            out.push(path.to_path_buf());
        }
    }
    let mut files = Vec::new();
    for p in paths {
        walk(p, &mut files);
    }
    files
        .into_iter()
        .filter_map(|f| std::fs::read_to_string(&f).ok().map(|t| (f, t)))
        .collect()
}

// -------------------------------------------------------------------------------------------
// The command
// -------------------------------------------------------------------------------------------

const USAGE: &str = "Usage: mn fuzz [--seed N] [--seconds N | --runs N] [--jobs N] [--corpus <path>]... [--out <dir>] [--phase-seconds N] [--stack-kib N]\n       mn fuzz --replay <file.mn | dir>...";

/// `mn fuzz`: returns the exit status, 1 when a failure was found, 2 on a usage error.
pub fn main(args: &[String]) -> i32 {
    let mut seed = 0u64;
    let mut runs = None;
    let mut seconds = None;
    let mut jobs = 1usize;
    let mut corpus = Vec::new();
    let mut out = PathBuf::from("fuzz-out");
    let mut phases = Phases::default();
    let mut replay = Vec::new();
    let mut is_worker = false;
    let mut replaying = false;
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        let mut number = |what: &str| -> Result<u64, String> {
            rest.next()
                .and_then(|n| n.parse::<u64>().ok())
                .ok_or_else(|| format!("error: `{}` takes a number", what))
        };
        let parsed = match a.as_str() {
            "--seed" => number(a).map(|n| seed = n),
            "--runs" => number(a).map(|n| runs = Some(n)),
            "--seconds" => number(a).map(|n| seconds = Some(n)),
            "--jobs" => number(a).map(|n| jobs = (n as usize).max(1)),
            "--phase-seconds" => number(a).map(|n| phases.budget = Duration::from_secs(n.max(1))),
            "--stack-kib" => number(a).map(|n| phases.stack = (n as usize).max(64) << 10),
            "--corpus" => match rest.next() {
                Some(p) => {
                    corpus.push(PathBuf::from(p));
                    Ok(())
                }
                None => Err("error: `--corpus` takes a path".to_string()),
            },
            "--out" => match rest.next() {
                Some(p) => {
                    out = PathBuf::from(p);
                    Ok(())
                }
                None => Err("error: `--out` takes a directory".to_string()),
            },
            "--worker" => {
                is_worker = true;
                Ok(())
            }
            "--replay" => {
                replaying = true;
                Ok(())
            }
            other if replaying && !other.starts_with('-') => {
                replay.push(PathBuf::from(other));
                Ok(())
            }
            other => Err(format!("error: `mn fuzz` does not know `{}`", other)),
        };
        if let Err(e) = parsed {
            eprintln!("{}\n{}", e, USAGE);
            return 2;
        }
    }
    if is_worker {
        phases.plant = plant_from_env();
        return worker(phases);
    }
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!("error: cannot find the mn executable: {}", e);
            return 2;
        }
    };
    if replaying {
        return replay_files(&exe, &phases, &replay);
    }
    let corpus_paths = if corpus.is_empty() {
        SEED_DIRS.iter().map(PathBuf::from).collect()
    } else {
        corpus
    };
    let seeds: Vec<String> = seed_corpus(&corpus_paths)
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    if seeds.is_empty() {
        eprintln!(
            "error: no seed files: run `mn fuzz` from the repository's root, or pass `--corpus`"
        );
        return 2;
    }
    let opts = Options {
        seed,
        runs,
        seconds: seconds.unwrap_or(60),
        jobs,
        out,
        phases,
        shallow: false,
    };
    fuzz(&exe, &opts, &seeds)
}

fn fuzz(exe: &Path, opts: &Options, seeds: &[String]) -> i32 {
    let start = Instant::now();
    let deadline = start + Duration::from_secs(opts.seconds);
    let known: Known = Mutex::new(BTreeMap::new());
    println!(
        "mn fuzz: seed {}, {} job(s), {} seed file(s), {}",
        opts.seed,
        opts.jobs,
        seeds.len(),
        match opts.runs {
            Some(r) => format!("{} runs", r),
            None => format!("{} s", opts.seconds),
        }
    );
    let results: Vec<Result<(Tally, usize), String>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..opts.jobs)
            .map(|job| {
                let known = &known;
                s.spawn(move || {
                    let mut worker = Worker::new(exe.to_path_buf(), &opts.phases);
                    campaign(job, opts, seeds, &mut worker, known, deadline)
                        .map(|t| (t, worker.restarts))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err("a job panicked".to_string()))
            })
            .collect()
    });
    let mut total = Tally::default();
    let mut restarts = 0;
    for r in results {
        match r {
            Ok((t, w)) => {
                total.runs += t.runs;
                for (p, (n, time)) in t.reached {
                    let entry = total.reached.entry(p).or_default();
                    entry.0 += n;
                    entry.1 += time;
                }
                restarts += w;
            }
            Err(e) => {
                eprintln!("error: {}", e);
                return 2;
            }
        }
    }
    let took = start.elapsed().as_secs_f64();
    let found: Vec<Found> = known
        .into_inner()
        .map(|k| k.into_values().flatten().collect())
        .unwrap_or_default();
    let mut summary = format!(
        "runs: {}\nseconds: {:.0}\nruns per second: {:.1}\nworker starts: {}\nfailures: {}\n",
        total.runs,
        took,
        total.runs as f64 / took.max(0.001),
        restarts,
        found.len()
    );
    summary.push_str("phases reached (runs, seconds spent):\n");
    for (p, (n, time)) in &total.reached {
        summary.push_str(&format!("  {}: {}, {:.1}\n", p, n, time.as_secs_f64()));
    }
    for f in &found {
        summary.push_str(&format!(
            "failure {}: {} ({} bytes)\n",
            slug(&f.failure),
            f.failure.key(),
            f.minimized.len()
        ));
    }
    print!("{}", summary);
    if std::fs::create_dir_all(&opts.out).is_ok() {
        let _ = std::fs::write(opts.out.join("summary.txt"), &summary);
    }
    i32::from(!found.is_empty())
}

/// Runs each file under `paths` through every phase, with a few edits, and prints each
/// failure. A file that hangs is not run again with the next edit, which would only hang too.
fn replay_files(exe: &Path, phases: &Phases, paths: &[PathBuf]) -> i32 {
    let files = seed_corpus(paths);
    if files.is_empty() {
        eprintln!("{}", USAGE);
        return 2;
    }
    let mut worker = Worker::new(exe.to_path_buf(), phases);
    let mut failed = 0;
    for (path, text) in &files {
        let mut seen = Vec::new();
        for edit in 0..4 {
            let case = Case {
                text: text.clone(),
                edit,
                thorough: true,
            };
            match worker.exec(&case) {
                Ok(outcome) => {
                    let hung = outcome.failures.iter().any(|f| f.kind == Kind::Hang);
                    for f in outcome.failures {
                        if !seen.contains(&f.key()) {
                            println!(
                                "{}: {} in {}: {}",
                                path.display(),
                                f.kind.name(),
                                f.phase,
                                f.message
                            );
                            seen.push(f.key());
                        }
                    }
                    if hung {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    return 2;
                }
            }
        }
        failed += usize::from(!seen.is_empty());
    }
    println!(
        "mn fuzz --replay: {} file(s), {} with a failure",
        files.len(),
        failed
    );
    i32::from(failed > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str = "module m\n\npub fn add(a: i32, b: i32) -> i32 = a + b\n\npub fn pick(c: bool, x: u8) -> u8 {\n  if c {\n    x\n  } else {\n    0\n  }\n}\n";

    fn phases(plant: Option<fn(&str)>) -> Phases {
        Phases {
            objects: false,
            plant,
            // Tests run in debug builds, with larger frames.
            stack: 8 << 20,
            ..Phases::default()
        }
    }

    /// One seed names one sequence of inputs, and another seed another.
    #[test]
    fn test_fuzzer_is_deterministic_for_a_seed() {
        let corpus = vec![
            SEED.to_string(),
            "module n\npub fn f() -> u8 = 7\n".to_string(),
        ];
        let cases = |seed: u64| -> Vec<Case> {
            let mut g = Generator::new(seed, 0, 1, &corpus, false);
            (0..200).map(|_| g.next_case()).collect()
        };
        assert_eq!(cases(7), cases(7));
        assert_ne!(cases(7), cases(8));
        // The seeds come first, as they are.
        assert_eq!(cases(7)[0].text, SEED);
        // Another job of the same seed makes other mutants.
        let mut other = Generator::new(7, 1, 2, &corpus, false);
        let theirs: Vec<Case> = (0..200).map(|_| other.next_case()).collect();
        assert_ne!(cases(7)[2..], theirs[1..199]);
        // The edit the incremental re-check makes depends on its seed alone.
        assert_eq!(edit_text(SEED, 3), edit_text(SEED, 3));
    }

    /// Each family of mutations changes its input, and does what its name says.
    #[test]
    fn test_each_mutation_family_produces_inputs() {
        let others = vec![
            SEED.to_string(),
            "module o\npub fn spliced_in(n: u64) -> u64 = n * 2\n".to_string(),
        ];
        for family in Family::ALL {
            let mut rng = Rng::new(11);
            let outs: Vec<String> = (0..300)
                .map(|_| apply(family, &mut rng, SEED, &others))
                .collect();
            let changed = outs.iter().filter(|o| o.as_str() != SEED).count();
            assert!(
                changed >= 200,
                "{:?} changed only {} of 300 inputs",
                family,
                changed
            );
            let any = |p: &dyn Fn(&str) -> bool| outs.iter().any(|o| p(o));
            match family {
                Family::Bytes => assert!(any(&|o| o.contains('\u{FFFD}'))),
                Family::NonAscii => {
                    assert!(any(&|o| o.contains('\u{4E2D}')));
                    assert!(any(&|o| o.contains('\u{202E}')));
                }
                Family::Tokens => {
                    let new_word = |o: &str| {
                        KEYWORDS
                            .iter()
                            .chain(&EXTRAS)
                            .any(|w| o.contains(w) && !SEED.contains(w))
                    };
                    assert!(outs.iter().filter(|o| new_word(o)).count() > 20);
                    assert!(any(&|o| o.len() < SEED.len()));
                }
                Family::Literals => {
                    assert!(any(
                        &|o| o.contains("340282366920938463463374607431768211455")
                    ));
                    assert!(any(
                        &|o| o.contains("170141183460469231731687303715884105728")
                    ));
                    assert!(any(&|o| o
                        .split(|c: char| !c.is_ascii_digit())
                        .any(|d| d.len() > 60)));
                }
                Family::Lines => assert!(any(&|o| o.lines().count() != SEED.lines().count())),
                Family::Splice => assert!(any(&|o| o.contains("spliced_in"))),
                Family::Nesting => {
                    assert!(any(&|o| o.contains(&"(".repeat(1000))));
                    assert!(any(&|o| o.contains(&"if x == 0 { 0 } else ".repeat(200))));
                    assert!(any(&|o| o.contains(&"if true { ".repeat(64))));
                    assert!(any(&|o| o.contains(&"while x > 0 { ".repeat(64))));
                }
            }
        }
    }

    /// Every keyword and operator the token mutations insert is one token of the lexer's.
    #[test]
    fn test_token_spellings_are_the_lexers_own() {
        for word in KEYWORDS.iter().chain(&PUNCTUATION) {
            let (tokens, errors) = mente_syntax::lex(word);
            assert!(errors.is_empty(), "`{}` does not lex: {:?}", word, errors);
            assert_eq!(tokens.len(), 2, "`{}` is not one token: {:?}", word, tokens);
        }
        for word in KEYWORDS {
            let (tokens, _) = mente_syntax::lex(word);
            assert!(
                !matches!(tokens[0].kind, TokenKind::Ident(_)),
                "`{}` is a name",
                word
            );
        }
    }

    /// The minimizer keeps the failure while it shrinks the input, down to what the failure
    /// needs.
    #[test]
    fn test_minimizer_keeps_the_failure_while_shrinking() {
        let input = format!("{}\npub fn bad() -> u8 = (((7)))\n{}", SEED, SEED);
        let mut calls = 0;
        let mut fails = |t: &str| {
            calls += 1;
            t.contains("bad") && t.contains("(((")
        };
        let small = minimize(&input, &mut fails);
        assert!(
            small.contains("bad") && small.contains("((("),
            "{:?}",
            small
        );
        assert!(
            small.len() <= "bad(((".len() + 1,
            "{:?} is not minimal",
            small
        );
        assert!(calls < 2000, "{} calls", calls);
        // A failure that needs the whole input keeps the whole input.
        let all = minimize("abc", &mut |t| t == "abc");
        assert_eq!(all, "abc");
    }

    fn planted_panic(src: &str) {
        if src.contains("340282366920938463463374607431768211455") {
            panic!("planted: u128's maximum");
        }
    }

    /// A planted panic in a test-only phase is found from the seeds, reported with its
    /// message and place, minimized, and written once.
    #[test]
    fn test_fuzzer_finds_and_minimizes_a_planted_panic() {
        let out = std::env::temp_dir().join(format!("mente-fuzz-planted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let opts = Options {
            seed: 5,
            runs: Some(1000),
            seconds: 60,
            jobs: 1,
            out: out.clone(),
            phases: phases(Some(planted_panic)),
            shallow: true,
        };
        let corpus = vec![SEED.to_string()];
        let known: Known = Mutex::new(BTreeMap::new());
        let mut exec = InProcess(opts.phases);
        let tally = campaign(0, &opts, &corpus, &mut exec, &known, Instant::now()).unwrap();
        assert_eq!(tally.runs, 1000);
        let found: Vec<Found> = known
            .into_inner()
            .unwrap()
            .into_values()
            .flatten()
            .collect();
        let planted: Vec<&Found> = found
            .iter()
            .filter(|f| f.failure.phase == "planted")
            .collect();
        assert_eq!(planted.len(), 1, "found {:?}", found);
        let f = planted[0];
        assert_eq!(f.failure.kind, Kind::Panic);
        assert!(
            f.failure
                .message
                .starts_with("planted: u128's maximum [at "),
            "{}",
            f.failure.message
        );
        assert!(
            f.failure.message.contains("fuzz.rs"),
            "{}",
            f.failure.message
        );
        assert!(f
            .minimized
            .contains("340282366920938463463374607431768211455"));
        assert!(f.minimized.len() < 48, "not minimized: {:?}", f.minimized);
        let name = slug(&f.failure);
        assert_eq!(
            std::fs::read_to_string(out.join(format!("{}.mn", name))).unwrap(),
            f.minimized
        );
        let about = std::fs::read_to_string(out.join(format!("{}.txt", name))).unwrap();
        assert!(
            about.contains("phase: planted") && about.contains("kind: panic"),
            "{}",
            about
        );
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A module that checks reaches every phase, the objects and both interpreters included,
    /// and fails none; one that does not check stops before the lowering, and fails none.
    #[test]
    fn test_a_clean_module_reaches_every_phase_without_a_failure() {
        let mut exec = InProcess(Phases {
            stack: 8 << 20,
            ..Phases::default()
        });
        let case = Case {
            text: SEED.to_string(),
            edit: 1,
            thorough: true,
        };
        let outcome = exec.exec(&case).unwrap();
        assert_eq!(outcome.failures, Vec::new());
        for phase in [
            "lex",
            "parse-bodies",
            "check",
            "render",
            "incremental",
            "tier1",
            "lower",
            "dump-mir",
            "object x86_64-unknown-linux-gnu",
            "object aarch64-unknown-linux-gnu",
            "object x86_64-pc-windows-msvc",
            "interp",
            "mir",
            "mir-without-proofs",
        ] {
            assert!(
                outcome.reached(phase),
                "{} not reached: {:?}",
                phase,
                outcome.reached
            );
        }
        let broken = Case {
            text: "module m\npub fn f() -> bool = 1\n".to_string(),
            edit: 1,
            thorough: false,
        };
        let outcome = InProcess(phases(None)).exec(&broken).unwrap();
        assert_eq!(outcome.failures, Vec::new());
        assert!(outcome.reached("check") && !outcome.reached("lower"));
    }

    /// Failures that meet one bug share a key: a panic by its place, a hang by its phase, and
    /// the rest by a message without its numbers and names.
    #[test]
    fn test_failures_of_one_bug_share_a_key() {
        let f = |phase: &str, kind, message: &str| Failure {
            phase: phase.into(),
            kind,
            message: message.into(),
        };
        assert_eq!(
            f(
                "check",
                Kind::Panic,
                "attempt to add with overflow [at a.rs:1:2]"
            )
            .key(),
            f(
                "tier1",
                Kind::Panic,
                "attempt to add with overflow [at a.rs:1:2]"
            )
            .key()
        );
        assert_eq!(
            f("lower", Kind::NotLowered, "fn foo: an if beyond 128 bits").key(),
            f("lower", Kind::NotLowered, "fn bar: an if beyond 128 bits").key()
        );
        assert_eq!(
            f(
                "mir",
                Kind::BrokenMir,
                "broken MIR: the exact v1 left its type"
            )
            .key(),
            f(
                "mir",
                Kind::BrokenMir,
                "broken MIR: the exact v12 left its type"
            )
            .key()
        );
        assert_ne!(
            f("mir", Kind::BrokenMir, "broken MIR: x").key(),
            f("mir-without-proofs", Kind::BrokenMir, "broken MIR: x").key()
        );
        assert_eq!(
            f("lower", Kind::NotLowered, "fn e: a value of no type").key(),
            f(
                "lower-without-proofs",
                Kind::NotLowered,
                "fn e: a value of no type"
            )
            .key()
        );
        assert_eq!(
            f(
                "incremental",
                Kind::Disagrees,
                &format!("{}: [E0111 at 2:1], []", RE_CHECK)
            )
            .key(),
            f(
                "incremental",
                Kind::Disagrees,
                &format!("{}: [], [E0216 at 5:3]", RE_CHECK)
            )
            .key()
        );
        assert_ne!(
            f(
                "incremental",
                Kind::Disagrees,
                &format!("{}: [], []", RE_CHECK)
            )
            .key(),
            f(
                "incremental",
                Kind::Disagrees,
                &format!("{}: [], []", FIRST_CHECK)
            )
            .key()
        );
        assert_eq!(
            f("parse", Kind::Hang, "took 12000 ms").key(),
            f("parse", Kind::Hang, "no answer within 13 s").key()
        );
        assert_eq!(unescape(&escape("a\tb\nc\\d")), "a\tb\nc\\d");
    }
}
