//! The Tier-1 re-check protocol of `ROADMAP.md`: the generated module, random single-token
//! edits inside function bodies, and the ignored test `tier1_recheck_protocol` that times the
//! session `mn check --watch` runs on them.
//!
//! The generated module and the edits are those of the M0b spike (`spikes/tier1-latency`),
//! written against the compiler's own lexer and parser.

use std::env;
use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

use mente_sema::incremental::Session;
use mente_syntax::{parse_module, ItemDecl, Token, TokenKind};

/// The target of `ROADMAP.md`: a 99th percentile under 5 ms.
const TARGET_MICROS: f64 = 5000.0;

/// The protocol's generated module: its functions, and the obligations Tier 1 proves in each.
const FUNCTIONS: usize = 200;
const OBLIGATIONS: usize = 20;

/// The seed of the generated module and of the edits, the spike's.
const SEED: u64 = 0x5eed;

/// A small deterministic generator (xorshift64).
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
}

/// A module of `functions` functions with [`OBLIGATIONS`] obligations each: loop bounds, array
/// indexing and a chain of `requires`. Each function walks part of an array twice, then calls
/// the previous function on a narrower range, whose `requires` it must establish; the first
/// calls the last, so every function has the same shape and the same obligations.
fn generated_module(functions: usize, seed: u64) -> String {
    let mut r = Rng(seed | 1);
    let mut out = String::from("module generated.tier1\n\n");
    for k in 0..functions {
        let callee = (k + functions - 1) % functions;
        let c1 = r.below(2001) as i64 - 1000;
        let c2 = r.below(2001) as i64 - 1000;
        out.push_str(&format!(
            "/// Link {k} of the chain.\n\
             fn f{k}(a: [64]i32, lo: usize[0..64], hi: usize[0..=64]) -> usize[0..=64]\n    \
                 requires lo < hi\n    \
                 ensures lo <= result and result <= hi\n\
             {{\n    \
                 var i: usize = lo\n    \
                 while i < hi invariant lo <= i {{\n        \
                     guard a[i] != {c1} else {{ break }}\n        \
                     i += 1\n    \
                 }}\n    \
                 var j: usize = hi\n    \
                 while j > lo invariant j <= hi {{\n        \
                     j -= 1\n        \
                     guard a[j] != {c2} else {{ break }}\n    \
                 }}\n    \
                 guard lo + 1 < hi else {{ return lo }}\n    \
                 return f{callee}(a, lo + 1, hi)\n\
             }}\n\n"
        ));
    }
    out
}

/// A function body: its tokens, with spans in the module, and the names of the function's
/// parameters and of the bindings the body declares.
struct Body {
    tokens: Vec<Token>,
    names: Vec<String>,
}

/// The bodies of `src`'s functions, from the token after the `{` or `=` that opens each.
fn bodies(src: &str) -> Vec<Body> {
    let Ok(module) = parse_module(src) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in &module.items {
        let ItemDecl::Fn(f) = item else { continue };
        let mut depth = 0i32;
        let open = f.tokens.iter().position(|t| {
            match t.kind {
                TokenKind::LParen | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBracket => depth -= 1,
                _ => {}
            }
            depth == 0
                && (t.kind == TokenKind::LBrace || (f.is_expr_body && t.kind == TokenKind::Eq))
        });
        let Some(open) = open else { continue };
        let tokens = f.tokens[open + 1..].to_vec();
        let mut names: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
        for pair in tokens.windows(2) {
            if let (TokenKind::Val | TokenKind::Var, TokenKind::Ident(n)) =
                (&pair[0].kind, &pair[1].kind)
            {
                if !names.contains(n) {
                    names.push(n.clone());
                }
            }
        }
        out.push(Body { tokens, names });
    }
    out
}

/// One single-token edit: `src[start..end]` replaced with `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Edit {
    start: usize,
    end: usize,
    text: String,
}

const COMPARISONS: [&str; 6] = ["<", "<=", ">", ">=", "==", "!="];

/// A random single-token edit in `body`: an integer literal, a comparison, `+` or `-` (also in
/// `+=` and `-=`), or a parameter's or binding's name replaced by another of its kind.
fn random_edit(src: &str, body: &Body, r: &mut Rng) -> Option<Edit> {
    let candidates: Vec<&Token> = body
        .tokens
        .iter()
        .filter(|t| match &t.kind {
            TokenKind::IntLit(..)
            | TokenKind::Lt
            | TokenKind::Le
            | TokenKind::Gt
            | TokenKind::Ge
            | TokenKind::EqEq
            | TokenKind::NotEq
            | TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::PlusEq
            | TokenKind::MinusEq => true,
            TokenKind::Ident(n) => body.names.len() > 1 && body.names.contains(n),
            _ => false,
        })
        .collect();
    let t = candidates.get(r.below(candidates.len()))?;
    let old = &src[t.span.start..t.span.end];
    let text = match &t.kind {
        TokenKind::IntLit(v, _) => [0, 1, 2, 63, 64, 65, v.saturating_sub(1), v + 1]
            .iter()
            .map(u128::to_string)
            .filter(|c| c != old)
            .nth(r.below(6))?,
        TokenKind::Plus => String::from("-"),
        TokenKind::Minus => String::from("+"),
        TokenKind::PlusEq => String::from("-="),
        TokenKind::MinusEq => String::from("+="),
        TokenKind::Ident(_) => {
            let others: Vec<&String> = body.names.iter().filter(|n| *n != old).collect();
            others[r.below(others.len())].clone()
        }
        _ => COMPARISONS
            .iter()
            .filter(|c| **c != old)
            .nth(r.below(5))?
            .to_string(),
    };
    Some(Edit {
        start: t.span.start,
        end: t.span.end,
        text,
    })
}

/// What timing one module's edits gave, in microseconds.
struct Measured {
    median: f64,
    p99: f64,
    /// Timed edits after which the session checked a function's names, types and Tier 1 again.
    rechecked: usize,
    /// Timed edits after which it checked the whole module, which declares more than functions.
    whole: usize,
}

/// Times `edits` random edits to `src` after `warmup` untimed ones, each applied to one running
/// session and undone, untimed, before the next; `None` when no body has a token to edit.
fn measure(src: &str, warmup: usize, edits: usize, seed: u64) -> Option<Measured> {
    let bodies = bodies(src);
    if bodies.is_empty() {
        return None;
    }
    let mut session = Session::new(src);
    let mut r = Rng(seed | 1);
    let mut samples = Vec::with_capacity(edits);
    let (mut rechecked, mut whole) = (0, 0);
    for n in 0..warmup + edits {
        let e = (0..100).find_map(|_| random_edit(src, &bodies[r.below(bodies.len())], &mut r))?;
        let old = &src[e.start..e.end];
        let t = Instant::now();
        session.edit(e.start, e.end, &e.text);
        black_box(session.diagnostics());
        let micros = t.elapsed().as_secs_f64() * 1e6;
        let work = session.work();
        session.edit(e.start, e.start + e.text.len(), old);
        if n >= warmup {
            samples.push(micros);
            rechecked += usize::from(work.checked > 0);
            whole += usize::from(work.whole);
        }
    }
    samples.sort_by(f64::total_cmp);
    Some(Measured {
        median: samples[samples.len() / 2],
        p99: samples[samples.len() * 99 / 100],
        rechecked,
        whole,
    })
}

/// The CPU the protocol ran on, where the system says.
fn cpu() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|info| {
            info.lines()
                .find_map(|l| l.strip_prefix("model name"))
                .map(|l| l.trim_start_matches([' ', '\t', ':']).to_string())
        })
        .unwrap_or_else(|| String::from("not reported"))
}

/// The Tier-1 re-check protocol in ROADMAP.md, on the generated module and every module in
/// `examples/`; with `MENTE_BENCH_ENFORCE=1`, a 99th percentile of 5 ms or more fails it:
/// `cargo test --release -p mente-cli -- --ignored --nocapture tier1_recheck_protocol`.
#[test]
#[ignore = "benchmark; run on demand"]
fn tier1_recheck_protocol() {
    let enforce = env::var_os("MENTE_BENCH_ENFORCE").is_some();
    let mut corpus = vec![(
        format!("generated, {FUNCTIONS} functions"),
        generated_module(FUNCTIONS, SEED),
    )];
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    for file in super::collect_mn_files(&examples) {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        corpus.push((name, fs::read_to_string(&file).unwrap()));
    }
    let mut table = String::from(
        "| Module | Median (µs) | 99th percentile (µs) | Function re-checked | Whole module checked |\n| :--- | ---: | ---: | ---: | ---: |\n",
    );
    let mut misses = Vec::new();
    for (name, src) in &corpus {
        let Some(m) = measure(src, 100, 1000, SEED) else {
            println!("{name}: no function body to edit");
            continue;
        };
        println!(
            "{name}: median {:.1} µs, p99 {:.1} µs; a function re-checked after {} of 1000 edits, the whole module after {}",
            m.median, m.p99, m.rechecked, m.whole
        );
        table.push_str(&format!(
            "| {name} | {:.1} | {:.1} | {} | {} |\n",
            m.median, m.p99, m.rechecked, m.whole
        ));
        if m.p99 >= TARGET_MICROS {
            misses.push(format!("{name}: p99 {:.1} µs", m.p99));
        }
    }
    let machine = format!("{}, {}", cpu(), env::consts::OS);
    println!("machine: {machine}");
    if let Some(summary) = env::var_os("GITHUB_STEP_SUMMARY") {
        let mut text = fs::read_to_string(&summary).unwrap_or_default();
        text.push_str(&format!(
            "## Tier-1 latency\n\n{table}\nMachine: {machine}. Target: a 99th percentile under 5 ms.\n"
        ));
        fs::write(&summary, text).unwrap();
    }
    assert!(
        !enforce || misses.is_empty(),
        "Tier 1 missed its 5 ms target:\n{}",
        misses.join("\n")
    );
}

#[test]
fn test_generated_functions_prove_their_obligations() {
    let src = generated_module(4, SEED);
    assert!(mente_sema::check_source(&src).is_ok());
    let reports = mente_sema::tier1_reports(&src);
    assert_eq!(reports.len(), 4);
    for (name, report) in reports {
        assert_eq!(report.obligations.len(), OBLIGATIONS, "{name}");
        assert!(report.obligations.iter().all(|o| o.proven), "{name}");
    }
}

#[test]
fn test_edits_change_one_token_of_a_body() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut sources = vec![generated_module(3, SEED)];
    for file in super::collect_mn_files(&examples) {
        sources.push(fs::read_to_string(file).unwrap());
    }
    let mut r = Rng(42);
    for src in &sources {
        let bodies = bodies(src);
        assert!(!bodies.is_empty());
        let before = mente_syntax::tokenize(src).len();
        for _ in 0..200 {
            let body = &bodies[r.below(bodies.len())];
            let Some(e) = random_edit(src, body, &mut r) else {
                continue;
            };
            assert!(body.tokens.iter().any(|t| t.span.start == e.start));
            assert_ne!(&src[e.start..e.end], e.text);
            let mut edited = src.clone();
            edited.replace_range(e.start..e.end, &e.text);
            assert_eq!(mente_syntax::tokenize(&edited).len(), before, "{e:?}");
        }
    }
}

#[test]
fn test_bodies_start_after_the_brace_or_the_equals_sign() {
    let src = "module m\n\nfn id(x: u8, ys: [2]u8) -> u8[0..=9] = x\n\nfn g(n: u8) -> u8\n    requires n == 1\n{\n    val k: u8 = n\n    k\n}\n";
    let bodies = bodies(src);
    let texts: Vec<Vec<&str>> = bodies
        .iter()
        .map(|b| {
            b.tokens
                .iter()
                .map(|t| &src[t.span.start..t.span.end])
                .collect()
        })
        .collect();
    assert_eq!(
        texts,
        vec![vec!["x"], vec!["val", "k", ":", "u8", "=", "n", "k", "}"]]
    );
    assert_eq!(bodies[0].names, vec!["x", "ys"]);
    assert_eq!(bodies[1].names, vec!["n", "k"]);
}

#[test]
fn test_a_session_edited_and_undone_matches_a_full_check() {
    let src = generated_module(4, SEED);
    let bodies = bodies(&src);
    let mut session = Session::new(&src);
    let mut r = Rng(7);
    for _ in 0..60 {
        let e = random_edit(&src, &bodies[r.below(bodies.len())], &mut r).unwrap();
        session.edit(e.start, e.end, &e.text);
        let mut edited = src.clone();
        edited.replace_range(e.start..e.end, &e.text);
        let full = mente_sema::check_source(&edited).err().unwrap_or_default();
        assert_eq!(session.diagnostics(), full.as_slice(), "{e:?}");
        let old = &src[e.start..e.end];
        session.edit(e.start, e.start + e.text.len(), old);
        assert!(session.diagnostics().is_empty(), "{e:?}");
    }
}
