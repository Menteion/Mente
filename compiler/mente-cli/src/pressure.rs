//! The pressure on the repository's files (Norman, 2026-11-17): how many merged pull requests
//! touched each file over a stretch of `main`'s history, so that a file many changes in flight
//! touch at once is seen, and the records it holds are made a kind of their own, or kept in one
//! file with the reason why, where [`crate::records`] declares both. It is a report, never a gate.
//!
//! It reads the history from standard input, as [`LOG`] prints it, and runs no git of its own, so
//! that it runs no program a repository's git configuration names.

use std::collections::BTreeMap;

use crate::records;

/// The command whose output `mn pressure` reads: each commit of `origin/main`'s first parents, a
/// NUL, its day, its parents, its marks and its subject apart by tabs, and each file it changed
/// from its first parent with its status. With every ref left out of the marks, the one mark git
/// can print is `grafted`, at the commit where a shallow history stops. `origin/main`, fetched first, is the history every clone agrees on,
/// where a local `main` may be old or missing. Paths are printed as they are (`core.quotePath`),
/// and a rename as the deletion and the addition it is, whatever the configuration says.
pub const LOG: &str = "git -c core.quotePath=false log --first-parent --diff-merges=first-parent --no-renames --name-status --decorate-refs-exclude=* --decorate-refs-exclude=HEAD --format=%x00%cs%x09%P%x09%D%x09%s origin/main";

/// What to run before [`LOG`], so that its history is whole and current.
pub const FETCH: &str = "git fetch origin main";

/// The subject's start that names a merged pull request, as the merge queue writes it.
const MERGED: &str = "Merge pull request #";

/// A pull request merged into `main`, and the files it changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Merge {
    /// The day it merged, `YYYY-MM-DD`.
    pub day: String,
    /// The pull request's number.
    pub pull: u32,
    /// The files it changed, deleted ones among them.
    pub files: Vec<String>,
    /// The files it deleted.
    pub deleted: Vec<String>,
}

/// Each pull request merged in `log`, the output of [`LOG`], in its order; a commit that merged no
/// pull request is passed over. A log that is empty, as git leaves one when it fails, a commit that
/// does not read as [`LOG`] prints one, and a merge with fewer than two parents, as a shallow clone
/// shows its oldest merge, are refused with what to do, so that no log is counted as another.
pub fn merges(log: &str) -> Result<Vec<Merge>, String> {
    let log = log.replace("\r\n", "\n");
    if log.trim().is_empty() {
        return Err(format!(
            "the log is empty, as git leaves it when it fails: run `{}`, then `{} | mn pressure`, in the repository",
            FETCH, LOG
        ));
    }
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
        let mut fields = head.splitn(4, '\t');
        let (Some(day), Some(parents), Some(marks), Some(subject)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(format!(
                "commit {} of the log starts with {:?}, not a day, its parents, its marks and a subject apart by tabs",
                n + 1,
                head
            ));
        };
        match marks {
            "" => {}
            "grafted" => {
                return Err(format!(
                    "commit {} of the log is grafted: the history is shallow there, and the merges before it are missing; run `git fetch --unshallow`, then the log again",
                    n + 1
                ))
            }
            _ => {
                return Err(format!(
                    "commit {} of the log is marked {:?}, which `{}` never prints",
                    n + 1,
                    marks,
                    LOG
                ))
            }
        }
        if !records::is_day(day) {
            return Err(format!(
                "commit {} of the log starts with {:?}, not a day",
                n + 1,
                head
            ));
        }
        let Some(pull) = pull_number(subject) else {
            continue;
        };
        let parents = parents.split_whitespace().count();
        if parents < 2 {
            return Err(format!(
                "commit {} of the log, the merge of pull request #{}, has {} parent{} where a merge has two: the history is shallow, or the pull request was squashed or rebased, and its files would be counted against nothing; run `git fetch --unshallow`, then the log again",
                n + 1,
                pull,
                parents,
                if parents == 1 { "" } else { "s" }
            ));
        }
        let mut files = Vec::new();
        let mut deleted = Vec::new();
        for line in lines.filter(|l| !l.is_empty()) {
            let Some((status, path)) = line
                .split_once('\t')
                .filter(|(s, _)| matches!(*s, "A" | "M" | "D" | "T"))
            else {
                return Err(format!(
                    "commit {} of the log holds {:?}, not a status and a path apart by a tab",
                    n + 1,
                    line
                ));
            };
            if status == "D" {
                deleted.push(path.to_string());
            }
            files.push(path.to_string());
        }
        out.push(Merge {
            day: day.to_string(),
            pull,
            files,
            deleted,
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
/// then by path, each with what it is among the records, and marked deleted where the newest of
/// those pull requests that changed it deleted it.
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
    let mut gone: BTreeMap<&str, bool> = BTreeMap::new();
    for m in &kept {
        for f in &m.files {
            gone.entry(f.as_str())
                .or_insert_with(|| m.deleted.contains(f));
        }
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
        let marks: Vec<String> = gone
            .get(file)
            .filter(|d| **d)
            .map(|_| "deleted".to_string())
            .into_iter()
            .chain(record(file))
            .collect();
        if marks.is_empty() {
            out.push_str(&format!("{}\n", line));
        } else {
            out.push_str(&format!("{}  ({})\n", line, marks.join("; ")));
        }
    }
    out
}
