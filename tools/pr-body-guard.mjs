// What the hook a Claude Code session of this repository runs before each shell command
// (tools/push-guard.mjs, from .claude/settings.json) decides about a request that creates or edits
// one of this repository's pull requests through the GitHub App's client, the one way a session
// reaches GitHub's API:
//
//   node .../gh-app.mjs api POST repos/Menteion/Mente/pulls --data-file <absolute path>
//   node .../gh-app.mjs api PATCH repos/Menteion/Mente/pulls/<number> --data-file <absolute path>
//
// It reads the request's JSON from the file the client will send, and refuses the command when the
// text fails the Commits check's rules, naming each problem: requestProblems in
// .github/scripts/lib/commits.mjs, the one definition the Commits check and
// .github/scripts/commits-body.mjs read too (root-cause-first; the escape
// pull-request-body-unchecked-locally: a body met the rules only in the required check).
//
// It is a deterrent at the session's hand, not the wall: the Commits check, a required check of
// `main`, reads every pull request's text from the API on every event, the merge queue's included.
// It reads the command's words as the shell would (raw-build.mjs, simpleCommands), and its known
// limits are those of a reading of text: a request through another client, through the issues API
// (`PATCH .../issues/<number>`, which also edits a pull request's body), or by a script that runs
// the client out of its sight, passes it. Within its reach it fails closed: a request file it cannot
// name, read or parse refuses the command. The workflow policy loads it and tests it, so this module
// starts no process; it reads the request file through the reader its caller gives.

import fs from "node:fs";
import path from "node:path";
import { requestProblems } from "../.github/scripts/lib/commits.mjs";
import { simpleCommands } from "./raw-build.mjs";

/// The routes of this repository's pull requests: `pulls`, which a POST creates one at, and
/// `pulls/<number>`, which a PATCH edits, with or without a leading slash, in any case, as GitHub
/// reads its owner and name.
const PULLS = /^\/?repos\/menteion\/mente\/pulls(\/\d+)?\/?$/i;
/// How deep the rule reads a command line given to another shell.
const DEPTH = 4;
const programOf = (value) => path.win32.basename(value).toLowerCase().replace(/\.exe$/, "");

const KEYWORDS = new Set(["then", "do", "else", "elif", "if", "while", "until", "!", "time", "&"]);
const PREFIXES = new Set(["command", "exec", "nohup", "sudo", "xargs", "env", "nice"]);

/// The index of the program of a simple command's words `v`: past the shell's keywords, its
/// variable assignments and the programs that run the rest of the words, with their options.
function programAt(v) {
  let i = 0;
  for (;;) {
    while (i < v.length && (KEYWORDS.has(v[i]) || /^[A-Za-z_][A-Za-z0-9_]*=/.test(v[i]))) i++;
    const p = programOf(v[i] ?? "");
    if (p === "timeout") {
      i++;
      while (i < v.length && v[i].startsWith("-")) i += ["-k", "--kill-after", "-s", "--signal"].includes(v[i]) ? 2 : 1;
      i++;
    } else if (PREFIXES.has(p)) {
      i++;
      while (i < v.length && (v[i].startsWith("-") || /^[A-Za-z_][A-Za-z0-9_]*=/.test(v[i]))) i += ["-u", "--unset", "-C", "--chdir", "-n"].includes(v[i]) ? 2 : 1;
    } else return i;
  }
}

/// Each call of the App's client in `text`, a simple command whose program is `node` and whose
/// script, its first word after node's options, is `gh-app.mjs`: the words after it and the command
/// as written, recursing into the shells it starts.
function clientCalls(text, depth = 0) {
  const found = [];
  for (const { text: line, words } of simpleCommands(String(text))) {
    const v = words.map((w) => w.value);
    const p = programAt(v);
    let at = p + 1;
    while (at < v.length && v[at].startsWith("-")) at++;
    if (programOf(v[p] ?? "") === "node" && programOf(v[at] ?? "") === "gh-app.mjs") found.push({ found: line.slice(words[0].start, words[words.length - 1].end), args: v.slice(at + 1) });
    if (depth >= DEPTH) continue;
    // A shell given a command line: bash's `-c` takes one word, PowerShell's `-Command` and cmd's
    // `/c` the rest of the line, as push.mjs reads them for git.
    for (let i = 0; i < v.length; i++) {
      const p = programOf(v[i]);
      const posix = ["bash", "sh", "zsh"].includes(p);
      const flag = posix ? /^-[a-z]*c$/ : ["powershell", "pwsh"].includes(p) ? /^-c(o(m(m(a(n(d)?)?)?)?)?)?$/i : p === "cmd" ? /^\/[a-z]*[ck]$/i : null;
      if (!flag) continue;
      const c = v.findIndex((w, k) => k > i && flag.test(w));
      if (c >= 0 && c + 1 < v.length) found.push(...clientCalls(posix || c === v.length - 2 ? v[c + 1] : line.slice(words[c + 1].start, words[words.length - 1].end), depth + 1));
      break;
    }
  }
  return found;
}

/// The file a request file's name reads as on `platform`, or why the rule cannot read it: a name the
/// shell would change (a variable, a home directory, a command), or one relative to a directory the
/// command may have left, refuses; on Windows, Git Bash's `/c/...` reads as `C:/...`.
function requestFile(name, platform) {
  if (/[$`%]/.test(name) || name.startsWith("~")) return { why: `names its request file through the shell, which this rule cannot read: name it by its literal absolute path` };
  if (platform === "win32") {
    const drive = /^\/([a-zA-Z])(\/.*)?$/.exec(name);
    if (drive) return { file: `${drive[1].toUpperCase()}:${drive[2] ?? "/"}` };
    if (/^[a-zA-Z]:[\\/]/.test(name)) return { file: name };
    return { why: `names its request file by a path that is not absolute on Windows: name it as C:/... or /c/...` };
  }
  return name.startsWith("/") ? { file: name } : { why: `names its request file by a relative path: name it by its absolute path` };
}

/// Why the rule refuses `command`, a request that creates or edits one of this repository's pull
/// requests with text that fails the Commits check's rules, or one whose text it cannot read; or
/// null. `readFile` reads a request file as text.
export function bodyRefusal(command, { readFile = (f) => fs.readFileSync(f, "utf8"), platform = process.platform } = {}) {
  for (const { found, args } of clientCalls(command)) {
    if (args[0] !== "api") continue;
    // The client's own reading: its method, upper-cased, its route, and the word after the first
    // `--data-file`, which it sends as the request's body.
    const rest = args.slice(1);
    const method = String(rest[0] ?? "").toUpperCase();
    const route = PULLS.exec(rest[1] ?? "");
    if (!route) continue;
    const creates = method === "POST" && route[1] === undefined;
    if (!creates && !(method === "PATCH" && route[1] !== undefined)) continue;
    const at = rest.indexOf("--data-file");
    if (at < 0 || at + 1 >= rest.length) continue;
    const name = rest[at + 1];
    const { file, why } = requestFile(name, platform);
    if (why) return `"${found}" ${why}`;
    // Written and sent on one line, the file may change after this rule reads it.
    const base = path.win32.basename(name).toLowerCase();
    if (String(command).toLowerCase().split(base).length > 2) return `"${found}" names its request file elsewhere on the same command line, so the file may change after this rule reads it: write the request in a command of its own, then send it`;
    let text;
    try {
      text = readFile(file);
    } catch (e) {
      return `"${found}" sends a request file this rule cannot read (${e.code ?? e.message})`;
    }
    let request;
    try {
      request = JSON.parse(String(text).replace(/^\uFEFF/, ""));
    } catch {
      return `"${found}" sends a request file that holds no JSON`;
    }
    const problems = requestProblems(request, { creates });
    if (problems.length > 0) return `"${found}" sends a pull request's text that fails the Commits check's rules, which it would meet on GitHub:\n${problems.map((p) => `- ${p}`).join("\n")}\nEdit ${name}, check it with node .github/scripts/commits-body.mjs ${name}, and send it again`;
  }
  return null;
}

/// What the settings' hook answers to `input`, its event as read from standard input: status 0 to let
/// the command run, or 2 to refuse it, with the words for the session. An event it cannot read
/// refuses; a tool's input with no command, a Monitor of a WebSocket, has nothing to read.
export function decide(input, options) {
  let event;
  try {
    event = JSON.parse(input);
  } catch (e) {
    return { status: 2, words: `the pull request body guard could not read its event, so the command did not run: ${e.message}` };
  }
  const command = event?.tool_input?.command;
  if (command === undefined && event?.tool_input?.ws !== undefined) return { status: 0, words: "" };
  if (typeof command !== "string") return { status: 2, words: "the pull request body guard found no command in its event, so the command did not run" };
  const why = bodyRefusal(command, options);
  return why ? { status: 2, words: `the pull request body guard refused it: ${why}.` } : { status: 0, words: "" };
}
