// The push guard: git's own pre-push hook of this repository (tools/git-hooks/pre-push, which
// core.hooksPath names and tools/quick.mjs, cargo.mjs and mn.mjs require), and the narrow rule of
// .claude/settings.json against bypassing it (tools/push-guard.mjs).
//
// The hook: git hands it, on standard input, every ref it is about to send, `<local ref> <local
// sha> <remote ref> <remote sha>` a line, whatever the configuration, alias, script or tool that
// started the push. It refuses unless the tree of every commit it would send is the one
// tools/quick.mjs last passed in this checkout, as the run recorded it in the checkout's git
// directory (local-limits.mjs, PASSED), or the commit is already on the remote's main, which it asks
// the remote itself (git ls-remote of the URL git gives it). A deletion sends nothing. A merge,
// cherry-pick, revert or rebase in progress refuses, and so do a record it cannot read, a remote it
// cannot ask, and an error of git: it fails closed. The slip it guards: on 2026-11-22 a merge of the evidence stack (#174, c94b58e) left two
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

/**
 * Whether `sha` is an ancestor of `ref`, or `ref` itself, in the repository of `dir`, both of
 * which it holds: false when it is not, and throws where git fails otherwise.
 */
function ancestor(dir, sha, ref) {
  try {
    execFileSync("git", ["-C", dir, "merge-base", "--is-ancestor", sha, ref], { stdio: ["ignore", "pipe", "pipe"] });
    return true;
  } catch (e) {
    if (e.status === 1) return false;
    throw e;
  }
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

/** How long the hook waits for the remote to say which commit is main. */
export const REMOTE_TIMEOUT_MS = 60000;

/**
 * The commit the remote at `url` names main, asked of the remote itself, never of a local ref such
 * as refs/remotes/origin/main, which any local command can move (the sixth review of #203):
 * { main } with its sha, or null when the remote has no main; or { elsewhere } with the URL
 * ls-remote would contact instead of `url`, the one git pushes to, which git hands the hook after
 * its push-side rewriting and ls-remote would rewrite again by url.<base>.insteadOf (the seventh
 * review). Within `timeoutMs`, with no prompt for credentials; throws where the remote cannot be
 * asked in time.
 */
function remoteMain(dir, url, timeoutMs) {
  const run = (args) =>
    execFileSync("git", ["-C", dir, ...args], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      env: { ...process.env, GIT_TERMINAL_PROMPT: "0" },
      timeout: timeoutMs,
    }).trim();
  const asked = run(["ls-remote", "--get-url", "--", url]);
  if (asked !== url) return { elsewhere: asked };
  const line = run(["ls-remote", "--", url, "refs/heads/main"])
    .split(/\r?\n/)
    .find((l) => /\trefs\/heads\/main$/.test(l));
  return { main: line ? line.split("\t")[0] : null };
}

/**
 * What the pre-push hook answers to `text`, the lines git gave it, in the checkout `dir`, pushing to
 * `url`, the URL git gave it: status 0 to let the push run, or 1 to refuse it, with the words for
 * whoever pushed. It waits for the remote `timeoutMs` at most. Throws where git fails, which the
 * hook refuses.
 */
export function prePush(text, dir, url, { timeoutMs = REMOTE_TIMEOUT_MS } = {}) {
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
  let passed = null;
  try {
    passed = fs.readFileSync(record, "utf8").trim();
  } catch (e) {
    if (e.code !== "ENOENT") throw e;
  }
  const valid = passed !== null && /^[0-9a-f]{40}([0-9a-f]{24})?$/.test(passed);
  const tree = (sha) => git(dir, "rev-parse", "--verify", "--quiet", `${sha}^{tree}`);
  const pending = valid ? sends.filter(({ sha }) => tree(sha) !== passed) : sends;
  if (pending.length === 0) return { status: 0, words: "" };
  // A commit reachable from the remote's main is already on main, which main's own CI and merge
  // queue hold: a tag of it, or a branch at it, needs no pass of its own (the fourth review of
  // #203). Which commit main is, the remote says, by the URL git pushes to.
  if (typeof url !== "string" || url === "") return { status: 1, words: "the pre-push hook refused: git gave it no remote's URL to ask for its main" };
  let asked;
  try {
    asked = remoteMain(dir, url, timeoutMs);
  } catch (e) {
    if (e.code === "ETIMEDOUT" || e.signal) return { status: 1, words: `the pre-push hook refused: ${url} did not say which commit is its main within ${Math.round(timeoutMs / 1000)} s, so it cannot tell whether ${pending[0].ref} is already on main` };
    return { status: 1, words: `the pre-push hook refused: it could not ask ${url} for its main, so it cannot tell whether ${pending[0].ref} is already on main: ${String(e.stderr || e.message).trim().split(/\r?\n/)[0]}` };
  }
  if (asked.elsewhere !== undefined) return { status: 1, words: `the pre-push hook refused: asking ${url} for its main would ask ${asked.elsewhere} instead, which a url.<base>.insteadOf rewrites it to, so it cannot tell what main is where the push goes` };
  const main = asked.main;
  let rest = pending;
  if (main !== null) {
    if (!exists(dir, `${main}^{commit}`)) return { status: 1, words: `the pre-push hook refused: the remote's main (${main.slice(0, 12)}) is not in this repository, so it cannot tell whether ${pending[0].ref} is already on main; fetch first, then push again` };
    rest = pending.filter(({ sha }) => !ancestor(dir, sha, main));
  }
  if (rest.length === 0) return { status: 0, words: "" };
  if (!valid) {
    // A branch from before the hook has a tools/quick.mjs that writes no record: running it cannot
    // help, so with no record the hook says what can. A record a later quick wrote still holds.
    if (!exists(dir, "HEAD:tools/git-hooks/pre-push")) return { status: 1, words: `the pre-push hook refused: this branch is from before the pre-push hook (#203), and its tools/quick.mjs writes no record of what it passed; merge main into this branch first, then run ${QUICK}, commit everything it read, and push again` };
    if (passed === null) return { status: 1, words: `the pre-push hook refused: no tree has passed the quick checks in this checkout since its last change; run ${QUICK}, commit everything it read, and push again` };
    return { status: 1, words: `the pre-push hook refused: ${record} holds no tree id; run ${QUICK} again` };
  }
  const { ref, sha } = rest[0];
  return { status: 1, words: `the pre-push hook refused: ${ref} (${sha.slice(0, 12)}) has the tree ${tree(sha).slice(0, 12)}, not the tree the quick checks last passed in this checkout (${passed.slice(0, 12)}); run ${QUICK} on what you push, commit everything it read, and push again` };
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
/** `git config`'s words that write. */
const CONFIG_WRITES = new Set(["--unset", "--unset-all", "--add", "--replace-all", "--rename-section", "--remove-section", "--edit", "-e", "set", "unset", "rename-section", "remove-section", "edit"]);
/** `git config`'s options that take the next word as their value. */
const CONFIG_VALUED = new Set(["-f", "--file", "--blob", "--type", "--default", "--comment", "--value"]);

/**
 * Whether `args`, the words after `git config`, write: a word that writes, or a key with a value.
 * A key alone, as `git config core.hooksPath`, or a word that only reads, reads.
 */
function configWrites(args) {
  if (args.some((w) => CONFIG_WRITES.has(w))) return true;
  if (args.some((w) => CONFIG_READS.has(w))) return false;
  const plain = [];
  for (let k = 0; k < args.length; k++) {
    if (CONFIG_VALUED.has(args[k])) k++;
    else if (!args[k].startsWith("-")) plain.push(args[k]);
  }
  return plain.length !== 1;
}

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
 * defined elsewhere, a lasting `git config` write of include.path, includeIf or alias.*, which it
 * lets through, a GIT_ variable set by an earlier command of the session's shell, and every spelling
 * it does not read: it reads the words named here, no others.
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
    if (sub === "config" && words.slice(j + 1).some((w) => HOOKS_KEY.test(w)) && configWrites(words.slice(j + 1))) {
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
