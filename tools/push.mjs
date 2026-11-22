// The push guard: git's own pre-push hook of this repository (tools/git-hooks/pre-push, which
// core.hooksPath names and tools/quick.mjs, cargo.mjs and mn.mjs require), and the narrow rule of
// .claude/settings.json against bypassing it (tools/push-guard.mjs).
//
// The hook: git hands it, on standard input, every ref it is about to send, `<local ref> <local
// sha> <remote ref> <remote sha>` a line, whatever the configuration, alias, script or tool that
// started the push. It refuses unless the tree of every commit it would send is the one
// tools/quick.mjs last passed in this checkout, as the run recorded it in the checkout's git
// directory (local-limits.mjs, PASSED). A deletion sends nothing. A merge, cherry-pick, revert or
// rebase in progress refuses, and so does a record it cannot read or an error of git: it fails
// closed. The slip it guards: on 2026-11-22 a merge of the evidence stack (#174, c94b58e) left two
// resolved files unstaged; the quick checks passed on the working tree, and the commit pushed
// lacked them.
//
// The rule, before a session's shell command runs, refuses the ways around the hook: a push with
// --no-verify, a git command that sets core.hooksPath for itself (-c, --config-env), and a push on a
// line that sets a GIT_ variable. Reading a command's text cannot tell what git will send (the
// second review of #203 found seven pushes a reading of refspecs, options and directories let
// through), so the rule reads only for these.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { PASSED } from "./local-limits.mjs";
import { simpleCommands } from "./raw-build.mjs";

/** What the guard asks a session to run when it refuses. */
export const QUICK = "node tools/quick.mjs";

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
 * What the pre-push hook answers to `text`, the lines git gave it, in the checkout `dir`: status 0
 * to let the push run, or 1 to refuse it, with the words for whoever pushed. Throws where git fails,
 * which the hook refuses.
 */
export function prePush(text, dir) {
  const sends = [];
  for (const line of String(text).split(/\r?\n/).filter((l) => l.trim() !== "")) {
    const parts = line.trim().split(/\s+/);
    if (parts.length !== 4 || !/^[0-9a-f]{40}([0-9a-f]{24})?$/.test(parts[1])) return { status: 1, words: `the pre-push hook cannot read the line git gave it: ${line}` };
    if (!/^0+$/.test(parts[1])) sends.push({ ref: parts[0], sha: parts[1] });
  }
  if (sends.length === 0) return { status: 0, words: "" };
  for (const [state, what] of [["MERGE_HEAD", "merge"], ["CHERRY_PICK_HEAD", "cherry-pick"], ["REVERT_HEAD", "revert"]]) {
    if (exists(dir, state)) return { status: 1, words: `the pre-push hook refused: a ${what} is in progress; finish it and commit, then run ${QUICK}` };
  }
  for (const state of ["rebase-merge", "rebase-apply"]) {
    if (fs.existsSync(git(dir, "rev-parse", "--path-format=absolute", "--git-path", state))) return { status: 1, words: `the pre-push hook refused: a rebase is in progress; finish it, then run ${QUICK}` };
  }
  const record = git(dir, "rev-parse", "--path-format=absolute", "--git-path", PASSED);
  let passed;
  try {
    passed = fs.readFileSync(record, "utf8").trim();
  } catch (e) {
    if (e.code === "ENOENT") return { status: 1, words: `the pre-push hook refused: no tree has passed the quick checks in this checkout since its last change; run ${QUICK}, commit everything it read, and push again` };
    throw e;
  }
  if (!/^[0-9a-f]{40}([0-9a-f]{24})?$/.test(passed)) return { status: 1, words: `the pre-push hook refused: ${record} holds no tree id; run ${QUICK} again` };
  for (const { ref, sha } of sends) {
    const tree = git(dir, "rev-parse", "--verify", "--quiet", `${sha}^{tree}`);
    if (tree !== passed) return { status: 1, words: `the pre-push hook refused: ${ref} (${sha.slice(0, 12)}) has the tree ${tree.slice(0, 12)}, not the tree the quick checks last passed in this checkout (${passed.slice(0, 12)}); run ${QUICK} on what you push, commit everything it read, and push again` };
  }
  return { status: 0, words: "" };
}

/** How deep the rule reads a command line given to another shell. */
const DEPTH = 4;
const KEYWORDS = new Set(["then", "do", "else", "elif", "if", "while", "until", "!", "time"]);
const PREFIXES = new Set(["command", "exec", "nohup", "sudo", "xargs"]);
const programOf = (value) => path.win32.basename(value).toLowerCase().replace(/\.exe$/, "");
/** A configuration key, in any case, whose setting turns the pre-push hook off or away. */
const HOOKS_KEY = /^core\.hookspath(=|$)/i;
/** A key whose setting brings in configuration from elsewhere, which may set the hooks' path. */
const INCLUDE_KEY = /^include(if\.[^=]*)?\.path(=|$)/i;
/** A key that defines an alias. */
const ALIAS_KEY = /^alias\./i;
/** git's options before its command that take the next word as their value. */
const GIT_VALUED = new Set(["-C", "-c", "--git-dir", "--work-tree", "--namespace", "--config-env", "--super-prefix", "--attr-source", "--list-cmds", "--exec-path"]);
/** `git config`'s words that only read. */
const CONFIG_READS = new Set(["--get", "--get-all", "--get-regexp", "--get-urlmatch", "--list", "-l", "get", "list"]);

/** Each git command of `text`: its words from the program on, recursing into shells it starts. */
function gitCommands(text, depth = 0) {
  const found = [];
  for (const { text: line, words } of simpleCommands(String(text))) {
    const v = (k) => words[k]?.value ?? "";
    let i = 0;
    for (;;) {
      while (i < words.length && (KEYWORDS.has(v(i)) || /^[A-Za-z_][A-Za-z0-9_]*=/.test(v(i)) || v(i) === "&")) i++;
      const q = programOf(v(i));
      if (q === "env") {
        i++;
        while (i < words.length && (v(i).startsWith("-") || /^[A-Za-z_][A-Za-z0-9_]*=/.test(v(i)))) i += ["-u", "--unset", "-C", "--chdir"].includes(v(i)) ? 2 : 1;
      } else if (q === "timeout") {
        i++;
        while (i < words.length && v(i).startsWith("-")) i += ["-k", "--kill-after", "-s", "--signal"].includes(v(i)) ? 2 : 1;
        i++;
      } else if (q === "nice") {
        i++;
        while (i < words.length && v(i).startsWith("-")) i += v(i) === "-n" ? 2 : 1;
      } else if (PREFIXES.has(q)) {
        i++;
        while (i < words.length && v(i).startsWith("-")) i++;
      } else break;
    }
    const p = programOf(v(i));
    const rest = words.slice(i);
    const slice = (from) => line.slice(words[from].start, words[words.length - 1].end);
    if (depth < DEPTH && ["bash", "sh", "zsh"].includes(p)) {
      const at = words.findIndex((w, k) => k > i && /^-[a-z]*c$/.test(w.value));
      if (at >= 0) found.push(...gitCommands(v(at + 1), depth + 1));
    } else if (depth < DEPTH && ["powershell", "pwsh"].includes(p)) {
      const at = words.findIndex((w, k) => k > i && /^-c(o(m(m(a(n(d)?)?)?)?)?)?$/i.test(w.value));
      if (at >= 0) found.push(...gitCommands(at === words.length - 2 ? v(at + 1) : slice(at + 1), depth + 1));
    } else if (depth < DEPTH && p === "cmd") {
      const at = words.findIndex((w, k) => k > i && /^\/[a-z]*[ck]$/i.test(w.value));
      if (at >= 0) found.push(...gitCommands(slice(at + 1), depth + 1));
    } else if (p === "git") {
      found.push({ found: slice(i), words: rest.map((w) => w.value), before: words.slice(0, i).map((w) => w.value) });
    }
  }
  return found;
}

/**
 * Why the rule refuses `command`, a way around the pre-push hook, or null. It reads the command's
 * words as the shell would (raw-build.mjs, simpleCommands), and is a deterrent, not the guard: the
 * hook, at the main checkout's absolute path, guards the slip, and CI gates whatever is pushed. Its
 * known limits: git's plumbing but send-pack, a libgit2 client, the GitHub API, a script or alias
 * defined elsewhere, and a GIT_ variable set by an earlier command of the session's shell.
 */
export function bypass(command) {
  const hook = "git's pre-push hook (tools/git-hooks/pre-push)";
  for (const { found, words, before } of gitCommands(command)) {
    const verify = words.find((w) => /^--no-ve/.test(w));
    if (verify) return `"${found}" passes ${verify}, which skips ${hook}`;
    let j = 1;
    while (j < words.length && words[j].startsWith("-")) {
      const a = words[j];
      const value = a === "-c" || a === "--config-env" ? words[j + 1] ?? "" : a.startsWith("--config-env=") ? a.slice("--config-env=".length) : /^-c./.test(a) ? a.slice(2) : null;
      if (value !== null) {
        if (HOOKS_KEY.test(value)) return `"${found}" sets core.hooksPath for itself, which turns ${hook} off`;
        if (INCLUDE_KEY.test(value)) return `"${found}" brings in configuration from elsewhere with -c, which may move ${hook} away`;
        if (ALIAS_KEY.test(value) && /push/i.test(value)) return `"${found}" defines an alias that pushes, out of sight of this rule`;
      }
      j += GIT_VALUED.has(a) ? 2 : 1;
    }
    const sub = words[j];
    if (sub === "send-pack") return `"${found}" pushes through git's plumbing, which runs no pre-push hook`;
    if (sub === "config" && words.slice(j + 1).some((w) => HOOKS_KEY.test(w)) && !words.slice(j + 1).some((w) => CONFIG_READS.has(w))) {
      return `"${found}" writes core.hooksPath, which names ${hook}; that is Norman's to set, in his own terminal (tools/local-limits.mjs, requireHooks)`;
    }
    if (sub !== "push") continue;
    if (before.some((w) => /^GIT_[A-Za-z0-9_]*=/i.test(w)) || /\bGIT_[A-Za-z0-9_]*=|(^|[^A-Za-z0-9_])env:GIT_/i.test(command)) return `"${found}" runs on a line that sets a GIT_ variable, which can move git, and its hooks, elsewhere; push on a line of its own`;
  }
  return null;
}

/**
 * What the settings' hook answers to `input`, its event as read from standard input: status 0 to
 * let the command run, or 2 to refuse it, with the words for the session. Any fault of its own
 * refuses; a tool's input with no command, a Monitor of a WebSocket, has nothing to read.
 */
export function decide(input) {
  let event;
  try {
    event = JSON.parse(input);
  } catch (e) {
    return { status: 2, words: `the push guard could not read its event, so the command did not run: ${e.message}` };
  }
  const command = event?.tool_input?.command;
  if (command === undefined && event?.tool_input?.ws !== undefined) return { status: 0, words: "" };
  if (typeof command !== "string") return { status: 2, words: "the push guard found no command in its event, so the command did not run" };
  const why = bypass(command);
  return why ? { status: 2, words: `the push guard refused it: ${why}.` } : { status: 0, words: "" };
}
