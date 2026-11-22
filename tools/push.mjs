// What the push guard decides (tools/push-guard.mjs, the hook of .claude/settings.json): a command
// that runs `git push` in a checkout of this repository runs only when every commit it would push
// has the tree tools/quick.mjs last passed in that checkout, as the run recorded it in its git
// directory (local-limits.mjs, PASSED). The slip it guards: on 2026-11-22 a merge of the evidence
// stack (#174, c94b58e) left two resolved files unstaged; the quick checks passed on the working
// tree, and the commit pushed lacked them. A merge, cherry-pick, revert or rebase in progress also
// refuses, and so does anything the guard cannot read: it fails closed.
//
// It reads the command's text, as the raw-build guard does (raw-build.mjs, simpleCommands): each
// simple command in order, `cd`, `pushd` and `Set-Location` moving the directory the later ones
// run in, `git -C <dir>` naming the repository, and `bash -c`, `sh -c`, `pwsh -Command` and
// `cmd /c` read within. It cannot see a push it does not read as one: a script or alias that
// pushes, a command built from a variable, `git` reached through another program, or a directory a
// subshell's `cd` leaves behind it.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { PASSED } from "./local-limits.mjs";
import { simpleCommands } from "./raw-build.mjs";

/** How deep the guard reads a command line given to another shell. */
const DEPTH = 4;
const KEYWORDS = new Set(["then", "do", "else", "elif", "if", "while", "until", "!", "time"]);
const PREFIXES = new Set(["command", "exec", "nohup", "sudo", "xargs"]);
/** git's options before its command that take the next word as their value. */
const GIT_VALUED = new Set(["-c", "--namespace", "--exec-path", "--config-env", "--super-prefix", "--list-cmds"]);
/** git's options that name a repository apart from its directory, which the guard does not follow. */
const GIT_ELSEWHERE = /^--(git-dir|work-tree)(=|$)/;
/** push's options that take the next word as their value. */
const PUSH_VALUED = new Set(["-o", "--push-option", "--repo", "--receive-pack", "--exec"]);
/** push's options that push refs beyond those named, for which no one tree can vouch. */
const PUSH_BEYOND = /^--(all|mirror|tags|prune|branches)$/;
/** What the guard asks a session to run when it refuses. */
export const QUICK = "node tools/quick.mjs";

const programOf = (value) => path.win32.basename(value).toLowerCase().replace(/\.exe$/, "");
/** A revision the guard cannot read for what it names: a variable or a substitution. */
const opaque = (value) => /[$`%]/.test(value);
/** A directory the guard cannot read for what it names: one opaque as a revision is, a home, or the last. */
const opaqueDir = (value) => opaque(value) || /^~|^-$/.test(value);

/** A directory as a shell names it, from `dir`: a path of git bash's /c/ form read as C:/. */
function moved(dir, to) {
  if (dir === null || opaqueDir(to)) return null;
  const drive = /^\/([a-zA-Z])(\/|$)/.exec(to);
  return path.resolve(dir, drive ? `${drive[1].toUpperCase()}:/${to.slice(3)}` : to);
}

/**
 * Each `git push` in `command`, run from `cwd`: { found, dir, args, elsewhere }, `found` as
 * written, `dir` the directory it runs in or null where the guard cannot tell, `args` the words
 * after `push`, and `elsewhere` set when an option names the repository apart from its directory.
 */
export function pushes(command, cwd) {
  const found = [];
  let dir = cwd;
  const read = (text, depth) => {
    for (const { text: line, words } of simpleCommands(String(text))) {
      const v = (k) => words[k]?.value ?? "";
      const slice = (from) => line.slice(words[from].start, words[words.length - 1].end);
      let i = 0;
      for (;;) {
        while (i < words.length && (KEYWORDS.has(v(i)) || /^[A-Za-z_][A-Za-z0-9_]*=/.test(v(i)) || v(i) === "&")) i++;
        const p = programOf(v(i));
        if (p === "env") {
          i++;
          while (i < words.length && (v(i).startsWith("-") || /^[A-Za-z_][A-Za-z0-9_]*=/.test(v(i)))) i += ["-u", "--unset", "-C", "--chdir"].includes(v(i)) ? 2 : 1;
          continue;
        }
        if (p === "timeout") {
          i++;
          while (i < words.length && v(i).startsWith("-")) i += ["-k", "--kill-after", "-s", "--signal"].includes(v(i)) ? 2 : 1;
          i++;
          continue;
        }
        if (p === "nice") {
          i++;
          while (i < words.length && v(i).startsWith("-")) i += v(i) === "-n" ? 2 : 1;
          continue;
        }
        if (PREFIXES.has(p)) {
          i++;
          while (i < words.length && v(i).startsWith("-")) i++;
          continue;
        }
        break;
      }
      if (i >= words.length) continue;
      const p = programOf(v(i));
      if (depth < DEPTH && ["bash", "sh", "zsh"].includes(p)) {
        const at = words.findIndex((w, k) => k > i && /^-[a-z]*c$/.test(w.value));
        if (at >= 0) read(v(at + 1), depth + 1);
        continue;
      }
      if (depth < DEPTH && ["powershell", "pwsh"].includes(p)) {
        const at = words.findIndex((w, k) => k > i && /^-c(o(m(m(a(n(d)?)?)?)?)?)?$/i.test(w.value));
        if (at >= 0) read(at === words.length - 2 ? v(at + 1) : slice(at + 1), depth + 1);
        continue;
      }
      if (depth < DEPTH && p === "cmd") {
        const at = words.findIndex((w, k) => k > i && /^\/[a-z]*[ck]$/i.test(w.value));
        if (at >= 0) read(slice(at + 1), depth + 1);
        continue;
      }
      if (["cd", "pushd", "chdir", "set-location", "sl"].includes(p)) {
        const to = words.slice(i + 1).find((w) => !w.value.startsWith("-") || w.value === "-");
        dir = to ? moved(dir, to.value) : null;
        continue;
      }
      if (p !== "git") continue;
      let at = dir;
      let elsewhere = false;
      let j = i + 1;
      while (j < words.length && v(j).startsWith("-")) {
        if (v(j) === "-C") {
          at = moved(at, v(j + 1));
          j += 2;
        } else if (GIT_ELSEWHERE.test(v(j))) {
          elsewhere = true;
          j += v(j).includes("=") ? 1 : 2;
        } else j += GIT_VALUED.has(v(j)) ? 2 : 1;
      }
      if (v(j) === "push") found.push({ found: slice(i), dir: at, args: words.slice(j + 1).map((w) => w.value), elsewhere });
    }
  };
  read(command, 0);
  return found;
}

/** git's output in `dir`, trimmed, or throws with git's own words. */
function git(dir, ...args) {
  return execFileSync("git", ["-C", dir, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
}

/** Whether `ref` names an object in the repository of `dir`. */
function exists(dir, ref) {
  try {
    git(dir, "rev-parse", "--verify", "--quiet", ref);
    return true;
  } catch {
    return false;
  }
}

/**
 * The commits `args`, push's words, would push from the repository of `dir`: HEAD when no refspec
 * is named, else each refspec's source, a deletion none; or { refused } with why.
 */
function sources(args) {
  const out = [];
  let positional = 0;
  let deleting = false;
  for (let k = 0; k < args.length; k++) {
    const a = args[k];
    if (a === "--") {
      for (const rest of args.slice(k + 1)) {
        if (positional++ > 0) out.push(rest);
      }
      break;
    }
    if (PUSH_BEYOND.test(a)) return { refused: `\`${a}\` pushes refs beyond those it names, for which no one tree quick passed can vouch` };
    if (a === "-d" || a === "--delete") deleting = true;
    else if (PUSH_VALUED.has(a)) k++;
    else if (a.startsWith("--repo=")) positional++;
    else if (!a.startsWith("-") && positional++ > 0) out.push(a);
  }
  if (deleting) return { commits: [] };
  const commits = [];
  for (const spec of out) {
    if (opaque(spec)) return { refused: `the guard cannot read the refspec \`${spec}\`` };
    const src = spec.replace(/^\+/, "").split(":")[0];
    if (src !== "") commits.push(src);
  }
  return { commits: out.length ? commits : ["HEAD"] };
}

/** Why the push `p` must not run, or null. Throws where git fails, which the caller refuses. */
function refusalOf(p) {
  if (p.elsewhere) return "it names its repository by --git-dir or --work-tree, which the guard does not follow; push with git -C <dir> instead";
  if (p.dir === null) return "the guard cannot tell which directory it runs in, since a `cd` before it names none it can read; write the directory out, as in git -C <dir> push";
  const top = git(p.dir, "rev-parse", "--show-toplevel");
  if (!fs.existsSync(path.join(top, "tools", "quick.mjs"))) return null;
  for (const [state, what] of [["MERGE_HEAD", "merge"], ["CHERRY_PICK_HEAD", "cherry-pick"], ["REVERT_HEAD", "revert"]]) {
    if (exists(top, state)) return `a ${what} is in progress in ${top}; finish it and commit, then run ${QUICK}`;
  }
  for (const state of ["rebase-merge", "rebase-apply"]) {
    if (fs.existsSync(git(top, "rev-parse", "--path-format=absolute", "--git-path", state))) return `a rebase is in progress in ${top}; finish it, then run ${QUICK}`;
  }
  const { refused, commits } = sources(p.args);
  if (refused) return refused;
  if (commits.length === 0) return null;
  const record = git(top, "rev-parse", "--path-format=absolute", "--git-path", PASSED);
  let passed;
  try {
    passed = fs.readFileSync(record, "utf8").trim();
  } catch (e) {
    if (e.code === "ENOENT") return `no tree has passed the quick checks in ${top} since its last change: run ${QUICK}, commit everything it read, and push again`;
    throw e;
  }
  if (!/^[0-9a-f]{40}([0-9a-f]{24})?$/.test(passed)) return `${record} holds no tree id; run ${QUICK} again`;
  for (const src of commits) {
    if (opaque(src)) return `the guard cannot read the commit \`${src}\``;
    const tree = git(top, "rev-parse", "--verify", "--quiet", `${src}^{tree}`);
    if (tree !== passed) return `it would push ${src}, whose tree ${tree.slice(0, 12)} is not the tree the quick checks last passed in ${top} (${passed.slice(0, 12)}): run ${QUICK} on what you push, commit everything it read, and push again`;
  }
  return null;
}

/**
 * What the hook answers to `input`, its event as read from standard input: status 0 to let the
 * command run, or 2 to refuse it, with the words for the session. Any fault of its own refuses.
 */
export function decide(input) {
  let event;
  try {
    event = JSON.parse(input);
  } catch (e) {
    return { status: 2, words: `the push guard could not read its event, so the command did not run: ${e.message}` };
  }
  const command = event?.tool_input?.command;
  if (typeof command !== "string") return { status: 2, words: "the push guard found no command in its event, so the command did not run" };
  if (!/push/i.test(command)) return { status: 0, words: "" };
  const cwd = typeof event.cwd === "string" ? event.cwd : process.cwd();
  for (const p of pushes(command, cwd)) {
    let why;
    try {
      why = refusalOf(p);
    } catch (e) {
      why = `git failed, so the guard could not tell what it would push: ${String(e.stderr || e.message).trim().split(/\r?\n/)[0]}`;
    }
    if (why) return { status: 2, words: `the push guard refused "${p.found}": ${why}.` };
  }
  return { status: 0, words: "" };
}
