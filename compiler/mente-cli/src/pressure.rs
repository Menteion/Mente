//! The pressure on the repository's files (Norman, 2026-11-17): how many merged pull requests
//! touched each file over a stretch of `main`'s history, so that a file many changes in flight
//! touch at once is seen, and the records it holds are made a kind of their own, or kept in one
//! file with the reason why, where [`crate::records`] declares both. It is a report, never a gate.
//!
//! It reads the history from standard input, as [`LOG`] prints it, and runs no git of its own, so
//! that it runs no program a repository's git configuration names.

use std::collections::BTreeMap;

use crate::records;

/// The command whose output `mn pressure` reads: each commit of `main`'s first parents, a NUL,
/// its day and subject apart by a tab, and the files it changed from its first parent.
pub const LOG: &str =
    "git log --first-parent --diff-merges=first-parent --name-only --format=%x00%cs%x09%s main";

/// The subject's start that names a merged pull request, as the merge queue writes it.
const MERGED: &str = "Merge pull request #";

/// A pull request merged into `main`, and the files it changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Merge {
    /// The day it merged, `YYYY-MM-DD`.
    pub day: String,
    /// The pull request's number.
    pub pull: u32,
    /// The files it changed.
    pub files: Vec<String>,
}

/// Each pull request merged in `log`, the output of [`LOG`], in its order; a commit that merged no
/// pull request is passed over. A commit that does not read as [`LOG`] prints one is refused with
/// its place, so that a log of another form is never counted as an empty one.
pub fn merges(log: &str) -> Result<Vec<Merge>, String> {
    let log = log.replace("\r\n", "\n");
    let mut commits = log.split('\0');
    if commits
        .next()
        .is_some_and(|before| !before.trim().is_empty())
    {
        return Err(format!(
            "the log does not start with a commit as `{}` prints one",
            LOG
        ));
    }
    let mut out = Vec::new();
    for (n, commit) in commits.enumerate() {
        let mut lines = commit.lines();
        let head = lines.next().unwrap_or_default();
        let Some((day, subject)) = head.split_once('\t').filter(|(d, _)| records::is_day(d)) else {
            return Err(format!(
                "commit {} of the log starts with {:?}, not a day and a subject apart by a tab",
                n + 1,
                head
            ));
        };
        let Some(pull) = pull_number(subject) else {
            continue;
        };
        out.push(Merge {
            day: day.to_string(),
            pull,
            files: lines
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
        });
    }
    Ok(out)
}

/// The number of the pull request a merge's subject names, `Merge pull request #170 from ...`.
fn pull_number(subject: &str) -> Option<u32> {
    let rest = subject.strip_prefix(MERGED)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let after = &rest[digits.len()..];
    (!digits.is_empty() && (after.is_empty() || after.starts_with(' ')))
        .then(|| digits.parse().ok())
        .flatten()
}

/// What `file` is among the records: in a kind's directory, or a file that still holds many
/// records, by [`records::KINDS`] and [`records::SHARED`].
fn record(file: &str) -> Option<String> {
    if let Some(shared) = records::SHARED.iter().find(|s| s.file == file) {
        return Some(format!("shared: {}", shared.records));
    }
    records::KINDS
        .iter()
        .find(|k| file.starts_with(&format!("{}/", k.dir)))
        .map(|k| format!("a kind: {}", k.record))
}

/// The report of `merges` on or after the day `since`, if one is given: how many pull requests
/// merged and over which days, and the `top` files the most of them changed, the most first and
/// then by path, each with what it is among the records.
pub fn report(merges: &[Merge], since: Option<&str>, top: usize) -> String {
    let kept: Vec<&Merge> = merges
        .iter()
        .filter(|m| since.is_none_or(|s| m.day.as_str() >= s))
        .collect();
    let Some(first) = kept.iter().map(|m| m.day.as_str()).min() else {
        return "No pull request merged in the log.\n".to_string();
    };
    let last = kept.iter().map(|m| m.day.as_str()).max().unwrap_or(first);
    let mut touched: BTreeMap<&str, usize> = BTreeMap::new();
    for m in &kept {
        let mut files: Vec<&str> = m.files.iter().map(String::as_str).collect();
        files.sort_unstable();
        files.dedup();
        for f in files {
            *touched.entry(f).or_default() += 1;
        }
    }
    let mut ranked: Vec<(&str, usize)> = touched.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let width = ranked.first().map_or(1, |(_, n)| n.to_string().len());
    let mut out = format!(
        "Pull requests merged from {} to {}: {}.\nThe files the most of them changed:\n",
        first,
        last,
        kept.len()
    );
    for (file, n) in ranked.into_iter().take(top) {
        let line = format!("  {:>w$}  {}", n, file, w = width);
        match record(file) {
            Some(what) => out.push_str(&format!("{}  ({})\n", line, what)),
            None => out.push_str(&format!("{}\n", line)),
        }
    }
    out
}
