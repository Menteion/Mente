//! The work the compiler does on each program of the conformance corpus, in counts that are the
//! same on every machine, where seconds on a shared runner differ two to three times between the
//! machines it may be given: the steps Tier 1 takes over every function of the program
//! (`mente_sema::octagon::Steps`), the values the lowering makes of a program that passes every
//! check, and the steps the reference interpreter runs for a fixed plan of calls of it. Beside
//! them, how many obligations Tier 1 proves, its precision.
//!
//! `tests/golden/work.txt` holds each program's counts. Each count of work stays within [`BAND`]
//! times its golden value either way, or within [`FLOOR`] of it for a small one: a change that
//! makes the compiler work more on some program fails until it rewrites the line
//! (`MENTE_BLESS=1`) and says why in its pull request, and one that makes it work less rewrites it
//! too, so the band holds the new level. The obligations proven must equal the golden count
//! exactly, so that a change that proves less, as a closure that misses a relaxation does, fails.
//! A program whose count of work stands past [`OUTLIER`] times the corpus's median for that count
//! names on its line why it costs that much, as `# tier1: reason`, `# mir: reason` or
//! `# interp: reason`, which a rewrite keeps while the count stays past it, and drops after.
//!
//! What the counts do not hold: Tier 1's work outside the octagon's entries, its walks of the
//! syntax tree, the values it builds and the linear scans of a forget, of the bounds kept beyond
//! the matrix (`check_far`) and of a picked point's relations (`related_bounds`); the time one
//! interpreter step takes, which a step that does more makes longer; and the work of the lowering
//! and of Cranelift beyond the values MIR holds.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use mente_codegen::lower;
use mente_interp::Limits;

use super::generate::{self, CallPlan};

/// The counts of one program: Tier 1's steps over all its functions and the obligations it
/// proves, and, for a program that passes every check, the values its lowering makes and the
/// interpreter's steps for [`PLAN`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Work {
    pub tier1: u64,
    pub proven: u64,
    pub mir: Option<u64>,
    pub interp: Option<u64>,
}

/// Each program's golden counts, and the reason each count of work past [`OUTLIER`] gives, by its
/// index in [`TAGS`].
pub type Golden = BTreeMap<String, (Work, BTreeMap<usize, String>)>;

/// The calls whose interpreter steps are counted: a few of the three-path suite's.
pub const PLAN: CallPlan = CallPlan {
    random: 2,
    exhaustive: 16,
};

/// How far a count of work may move from its golden value either way, as a ratio: 3/2.
pub const BAND: (u64, u64) = (3, 2);

/// How far a small count of work may move from its golden value either way, for each count: the
/// band of a count this small is a few entries, values or steps, which any change to the compiler
/// moves.
pub const FLOOR: [u64; 3] = [4096, 32, 256];

/// How many times the corpus's median a count of work may stand before its line must say why,
/// the median of the programs that have the count at all. Measured on the corpus on 2026-11-22,
/// with the closure that visits only what an edge can change: Tier 1's highest program but one
/// stood at 56 times the median of 8,752 steps and `slots_review_generated_35.mn` at 1,579 times;
/// the MIR's highest at 29 times the median of 105 values; the interpreter's loops at a few
/// hundred times its median of 18 steps, and the programs that run to the step limit far past.
pub const OUTLIER: u64 = 64;

/// The interpreter's limits for the counted calls: the three-path suite's.
pub const LIMITS: Limits = Limits {
    max_steps: 1_000_000,
    max_array: 1 << 24,
};

/// The tag of each count of work, as a reason on a golden line names it.
pub const TAGS: [&str; 3] = ["tier1", "mir", "interp"];

const KINDS: [&str; 3] = [
    "Tier 1's steps",
    "the MIR's values",
    "the interpreter's steps",
];

/// The golden file of the counts.
pub fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/work.txt")
}

/// Every program of the conformance corpus, by its path under `conformance/`, with its text.
pub fn programs() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance");
    let mut out = Vec::new();
    let mut dirs = vec![root.clone()];
    while let Some(d) = dirs.pop() {
        for entry in fs::read_dir(&d).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "mn") {
                let name = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((name, fs::read_to_string(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// The counts of one program, `name` its path, which seeds its calls as the three-path suite's.
pub fn work_of(name: &str, src: &str) -> Work {
    let Ok(module) = mente_sema::check_program(src) else {
        let reports = mente_sema::tier1_reports(src);
        return Work {
            tier1: reports.iter().map(|(_, r)| r.steps).sum(),
            proven: reports
                .iter()
                .flat_map(|(_, r)| &r.obligations)
                .filter(|o| o.proven)
                .count() as u64,
            mir: None,
            interp: None,
        };
    };
    let functions = || module.functions.iter().flatten();
    let tier1 = functions().map(|f| f.steps).sum();
    let proven = functions()
        .flat_map(|f| &f.obligations)
        .filter(|o| o.proven)
        .count() as u64;
    let mir = lower::lower(&module)
        .ok()
        .map(|l| l.module.functions.iter().map(|f| f.ops.len() as u64).sum());
    let seed = (name.len() * 7919 + src.len()) as u64;
    let interp = generate::calls(src, &module, seed, PLAN)
        .into_iter()
        .map(|(f, args)| {
            let args = args.into_iter().map(|(_, v)| v).collect();
            mente_interp::call_counted(&module, &f, args, LIMITS).1
        })
        .sum();
    Work {
        tier1,
        proven,
        mir,
        interp: Some(interp),
    }
}

/// A golden line: the program, its counts, `-` for one it has none of, and its reasons.
pub fn line(name: &str, w: &Work, reasons: &BTreeMap<usize, String>) -> String {
    let n = |c: Option<u64>| c.map_or("-".to_string(), |c| c.to_string());
    let mut out = format!(
        "{} {} {} {} {}",
        name,
        w.tier1,
        w.proven,
        n(w.mir),
        n(w.interp)
    );
    for (kind, r) in reasons {
        out.push_str(&format!("  # {}: {}", TAGS[*kind], r));
    }
    out
}

/// The golden file read: each program's counts and reasons. A line not of the form
/// `<program> <tier1> <proven> <mir> <interp>  # <tag>: reason`, a reason of no count or two of
/// one, and a second line of one program are refused.
pub fn read_golden(text: &str) -> Golden {
    let mut out = Golden::new();
    for (k, raw) in text.lines().enumerate() {
        let at = format!("tests/golden/work.txt:{}", k + 1);
        let mut pieces = raw.split("  # ");
        let body = pieces.next().unwrap_or("");
        if body.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = body.split_whitespace().collect();
        let count = |s: &str| (s != "-").then(|| s.parse::<u64>().ok()).flatten();
        let well_formed = parts.len() == 5
            && parts[1..3].iter().all(|p| p.parse::<u64>().is_ok())
            && parts[3..]
                .iter()
                .all(|p| *p == "-" || p.parse::<u64>().is_ok());
        assert!(
            well_formed,
            "{}: not `<program> <tier1> <proven> <mir> <interp>  # <tag>: reason`: {}",
            at, raw
        );
        let mut reasons = BTreeMap::new();
        for piece in pieces {
            let tagged = piece
                .split_once(": ")
                .and_then(|(tag, r)| Some((TAGS.iter().position(|t| *t == tag)?, r.trim())));
            let Some((kind, reason)) = tagged.filter(|(_, r)| !r.is_empty()) else {
                panic!(
                    "{}: a reason not tagged with its count, as `# tier1: ...`, `# mir: ...` or `# interp: ...`: {}",
                    at, piece
                );
            };
            assert!(
                reasons.insert(kind, reason.to_string()).is_none(),
                "{}: two reasons for {}",
                at,
                KINDS[kind]
            );
        }
        let w = Work {
            tier1: parts[1].parse().unwrap_or(0),
            proven: parts[2].parse().unwrap_or(0),
            mir: count(parts[3]),
            interp: count(parts[4]),
        };
        let previous = out.insert(parts[0].to_string(), (w, reasons));
        assert!(previous.is_none(), "{}: a second line of {}", at, parts[0]);
    }
    out
}

/// Whether `now` lies within the band of `golden`, a count of work of kind `kind` (0 Tier 1, 1
/// MIR, 2 the interpreter).
pub fn within(kind: usize, golden: u64, now: u64) -> bool {
    let (num, den) = BAND;
    let high = (golden.saturating_mul(num) / den).max(golden.saturating_add(FLOOR[kind]));
    let low = (golden.saturating_mul(den) / num).min(golden.saturating_sub(FLOOR[kind]));
    (low..=high).contains(&now)
}

/// Each count of work of `w`, `None` where it has none.
pub fn counts(w: &Work) -> [Option<u64>; 3] {
    [Some(w.tier1), w.mir, w.interp]
}

/// The median of each count of work over the corpus, of the programs whose count is above 0: a
/// program Tier 1 does not prove, or whose calls take no step, says nothing of what a program
/// costs.
pub fn medians(all: &[(String, Work)]) -> [u64; 3] {
    let mut out = [0; 3];
    for (kind, m) in out.iter_mut().enumerate() {
        let mut v: Vec<u64> = all
            .iter()
            .filter_map(|(_, w)| counts(w)[kind])
            .filter(|c| *c > 0)
            .collect();
        v.sort_unstable();
        *m = v.get(v.len() / 2).copied().unwrap_or(0);
    }
    out
}

/// Whether each count of work of `w` stands past [`OUTLIER`] times its median.
fn outliers(w: &Work, medians: &[u64; 3]) -> [bool; 3] {
    let c = counts(w);
    std::array::from_fn(|kind| {
        c[kind].is_some_and(|n| n > medians[kind].saturating_mul(OUTLIER).max(1))
    })
}

/// What the rules find of the corpus's counts `all` against the golden ones: a count of work
/// outside its band, one side's count the other lacks, obligations proven other than the golden
/// count, a program the file does not list or one it lists that the corpus lacks, a count past
/// [`OUTLIER`] times the median without its reason, and a reason for a count that no longer is.
pub fn problems(all: &[(String, Work)], golden: &Golden) -> Vec<String> {
    let mut out = Vec::new();
    let medians = medians(all);
    let bless = "rewrite tests/golden/work.txt with MENTE_BLESS=1 and say why in the pull request";
    for (name, w) in all {
        let Some((g, reasons)) = golden.get(name) else {
            out.push(format!(
                "{}: no line in tests/golden/work.txt: {}",
                name, bless
            ));
            continue;
        };
        for (kind, (now, was)) in counts(w).into_iter().zip(counts(g)).enumerate() {
            match (now, was) {
                (Some(now), Some(was)) if !within(kind, was, now) => out.push(format!(
                    "{}: {} are {}, outside the band of {} the golden file holds: {}",
                    name, KINDS[kind], now, was, bless
                )),
                (Some(_), None) | (None, Some(_)) => out.push(format!(
                    "{}: {} are {:?}, where the golden file holds {:?}: {}",
                    name, KINDS[kind], now, was, bless
                )),
                _ => {}
            }
        }
        if w.proven != g.proven {
            out.push(format!(
                "{}: Tier 1 proves {} obligations, where the golden file holds {}: a change to its precision, which a closure that misses a relaxation makes too: {}",
                name, w.proven, g.proven, bless
            ));
        }
        for (kind, far) in outliers(w, &medians).into_iter().enumerate() {
            match (far, reasons.contains_key(&kind)) {
                (true, false) => out.push(format!(
                    "{}: {} are {}, past {} times the corpus's median of {}: say on its line in tests/golden/work.txt why it costs that much, as `  # {}: reason`",
                    name,
                    KINDS[kind],
                    counts(w)[kind].unwrap_or(0),
                    OUTLIER,
                    medians[kind],
                    TAGS[kind]
                )),
                (false, true) => out.push(format!(
                    "{}: its line gives a reason for {}, which stand within {} times the corpus's median: remove it, as MENTE_BLESS=1 does",
                    name, KINDS[kind], OUTLIER
                )),
                _ => {}
            }
        }
    }
    for name in golden.keys() {
        if !all.iter().any(|(n, _)| n == name) {
            out.push(format!(
                "tests/golden/work.txt lists {}, which is no program of the corpus: remove its line",
                name
            ));
        }
    }
    out
}

/// The golden file for the corpus's counts, keeping each program's reason for a count that still
/// stands past [`OUTLIER`] times its median, and dropping the others.
pub fn blessed(all: &[(String, Work)], golden: &Golden) -> String {
    let medians = medians(all);
    all.iter()
        .map(|(name, w)| {
            let far = outliers(w, &medians);
            let reasons: BTreeMap<usize, String> = golden
                .get(name)
                .map(|(_, r)| {
                    r.iter()
                        .filter(|(k, _)| far[**k])
                        .map(|(k, v)| (*k, v.clone()))
                        .collect()
                })
                .unwrap_or_default();
            line(name, w, &reasons) + "\n"
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each conformance program does the work its golden line states: every count of work within
    /// its band, the obligations proven exactly, every program of the corpus listed once and
    /// nothing else, and a count past [`OUTLIER`] times the corpus's median with its reason on its
    /// line. Under `MENTE_BLESS=1` it rewrites the file first, keeping each reason still needed,
    /// and then holds the outlier rule.
    #[test]
    fn test_each_conformance_program_does_the_work_its_golden_line_states() {
        let all: Vec<(String, Work)> = programs()
            .iter()
            .map(|(name, src)| (name.clone(), work_of(name, src)))
            .collect();
        let mut golden_text = fs::read_to_string(golden()).unwrap_or_default();
        if std::env::var_os("MENTE_BLESS").is_some() {
            golden_text = blessed(&all, &read_golden(&golden_text));
            fs::write(golden(), &golden_text).unwrap();
        }
        let found = problems(&all, &read_golden(&golden_text));
        assert!(found.is_empty(), "{}", found.join("\n"));
    }

    /// The rules over counts of their own: growth and shrinking past the band fail, a small
    /// count moves within its floor, a program unlisted or a line of no program fails, a count
    /// that appears or vanishes fails, obligations proven differ by one and fail, an outlier
    /// needs a reason of its own count, a reason of another count does not serve, a reason no
    /// longer needed fails, and a rewrite keeps a needed reason and drops the rest.
    #[test]
    fn test_the_work_rules_refuse_what_they_must() {
        let w = |tier1: u64, mir: Option<u64>, interp: Option<u64>| Work {
            tier1,
            proven: 40,
            mir,
            interp,
        };
        let corpus = |big: Work| {
            let mut all: Vec<(String, Work)> = (0..9)
                .map(|k| (format!("p{}.mn", k), w(100_000, Some(1000), Some(10_000))))
                .collect();
            all.push(("big.mn".to_string(), big));
            all
        };
        let none = Golden::new();
        let base = w(100_000, Some(1000), Some(10_000));
        let all = corpus(base);
        let golden = read_golden(&blessed(&all, &none));
        assert_eq!(problems(&all, &golden), Vec::<String>::new());
        // Past the band, either way, on each count.
        for moved in [
            w(150_001, Some(1000), Some(10_000)),
            w(66_000, Some(1000), Some(10_000)),
            w(100_000, Some(1501), Some(10_000)),
            w(100_000, Some(1000), Some(6_600)),
        ] {
            assert_eq!(problems(&corpus(moved), &golden).len(), 1, "{:?}", moved);
        }
        // Within the band, and a small count within its floor.
        assert!(problems(&corpus(w(149_999, Some(667), Some(14_999))), &golden).is_empty());
        let small = corpus(w(10, Some(3), Some(5)));
        let small_golden = read_golden(&blessed(&small, &none));
        assert!(problems(&corpus(w(4000, Some(30), Some(250))), &small_golden).is_empty());
        // A program unlisted, a line of no program, a count that appears or vanishes.
        let mut more = all.clone();
        more.push(("new.mn".to_string(), base));
        assert_eq!(problems(&more, &golden).len(), 1);
        assert_eq!(problems(&all[1..], &golden).len(), 1);
        assert_eq!(problems(&corpus(w(100_000, None, None)), &golden).len(), 2);
        // Obligations proven, one fewer or one more.
        for proven in [39, 41] {
            let moved = Work { proven, ..base };
            assert_eq!(problems(&corpus(moved), &golden).len(), 1, "{proven}");
        }
        // An outlier needs a reason of its own count, which a rewrite keeps.
        let far_steps = 100_000 * OUTLIER + 1;
        let far = corpus(w(far_steps, Some(1000), Some(10_000)));
        let unexplained = read_golden(&blessed(&far, &none));
        assert_eq!(problems(&far, &unexplained).len(), 1);
        let plain = format!("big.mn {} 40 1000 10000", far_steps);
        let with =
            |reason: &str| blessed(&far, &none).replace(&plain, &format!("{plain}  # {reason}"));
        let explained = read_golden(&with("tier1: nested loops over many arrays"));
        assert!(problems(&far, &explained).is_empty());
        assert_eq!(
            blessed(&far, &explained),
            with("tier1: nested loops over many arrays")
        );
        let elsewhere = read_golden(&with("interp: long loops"));
        assert_eq!(
            problems(&far, &elsewhere).len(),
            2,
            "a reason of another count serves"
        );
        // A reason no longer needed fails, and a rewrite drops it.
        let stale = read_golden(&blessed(&all, &none).replace(
            "big.mn 100000 40 1000 10000",
            "big.mn 100000 40 1000 10000  # tier1: once nested loops",
        ));
        assert_eq!(problems(&all, &stale).len(), 1);
        assert_eq!(blessed(&all, &stale), blessed(&all, &none));
    }

    /// The median is that of the counts above 0, the middle one of them; a count stands past
    /// [`OUTLIER`] times it only above that product, and a median of 0 makes every count above 1
    /// an outlier, never 1 itself.
    #[test]
    fn test_the_median_and_the_outlier_line() {
        let w = |tier1: u64, interp: Option<u64>| Work {
            tier1,
            proven: 0,
            mir: None,
            interp,
        };
        let all: Vec<(String, Work)> = [0, 0, 0, 10, 20, 30, 40, 50]
            .iter()
            .enumerate()
            .map(|(k, t)| (format!("p{k}.mn"), w(*t, Some(*t / 10))))
            .collect();
        assert_eq!(medians(&all), [30, 0, 3]);
        let line = [30, 0, 3];
        assert_eq!(outliers(&w(30 * OUTLIER, None), &line), [false; 3]);
        assert_eq!(
            outliers(&w(30 * OUTLIER + 1, None), &line),
            [true, false, false]
        );
        assert_eq!(outliers(&w(1, Some(1)), &[0, 0, 0]), [false; 3]);
        assert_eq!(outliers(&w(2, Some(2)), &[0, 0, 0]), [true, false, true]);
    }

    /// A golden line not of the form `<program> <tier1> <proven> <mir> <interp>` is refused.
    #[test]
    #[should_panic(
        expected = "tests/golden/work.txt:2: not `<program> <tier1> <proven> <mir> <interp>"
    )]
    fn test_a_misshapen_golden_line_is_refused() {
        read_golden("a.mn 1 2 3 4\nb.mn 1 2 3\n");
    }

    /// A second line of one program is refused, where a map would keep one of the two unseen.
    #[test]
    #[should_panic(expected = "tests/golden/work.txt:3: a second line of a.mn")]
    fn test_a_repeated_golden_line_is_refused() {
        read_golden("a.mn 1 2 3 4\nb.mn 1 2 3 4\na.mn 5 6 7 8\n");
    }

    /// A reason not tagged with one of the counts, or two of one count, is refused.
    #[test]
    fn test_a_reason_names_its_count() {
        for text in [
            "a.mn 1 2 3 4  # loops\n",
            "a.mn 1 2 3 4  # time: loops\n",
            "a.mn 1 2 3 4  # tier1: a  # tier1: b\n",
        ] {
            let refused = std::panic::catch_unwind(|| read_golden(text));
            assert!(refused.is_err(), "{text}");
        }
        let read = read_golden("a.mn 1 2 3 4  # tier1: a  # interp: b\n");
        assert_eq!(read["a.mn"].1.len(), 2);
    }
}
