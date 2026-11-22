// What the hook a Claude Code session of this repository runs before each shell command
// (tools/push-guard.mjs, from .claude/settings.json) decides about a request that sends a pull
// request's text through the GitHub App's client, the one way a session reaches GitHub's API:
//
//   node .../gh-app.mjs api POST repos/Menteion/Mente/pulls --data-file C:/.../request.json
//   node .../gh-app.mjs api PATCH repos/Menteion/Mente/pulls/<number> --data-file C:/.../request.json
//   node .../gh-app.mjs api POST graphql --data-file C:/.../mutation.json
//
// It reads the request's JSON from the file the client will send, and refuses the command when the
// text fails the Commits check's rules, naming each problem: requestProblems in
// .github/scripts/lib/commits.mjs, the one definition the Commits check and
// .github/scripts/commits-body.mjs read too (root-cause-first; the escape
// pull-request-body-unchecked-locally: a body met the rules only in the required check).
//
// It reads the route as the client builds its URL and the API reads it (requestPath): a full URL or
// a path, its query and fragment dropped, dot segments resolved, decoded and lower-cased. Of GraphQL
// it reads a request whose query names `createPullRequest` or `updatePullRequest`, and the title and
// body it sends, inline or in its variables (graphqlTexts).
//
// It is a deterrent at the session's hand, not the wall: the Commits check, a required check of
// `main`, reads every pull request's text from the API on every event, the merge queue's included.
// It reads the command's words as the shell would (raw-build.mjs, simpleCommands), and its known
// limits are those of a reading of text: a request through another client, through the issues API
// (`PATCH .../issues/<number>`, which also edits a pull request's body), or by a script that runs the
// client out of its sight, passes it. Within its reach it fails closed: a call that sends a file and
// names a word of itself through the shell, or a request file it cannot name, read or parse, refuses
// the command. The workflow policy loads it and tests it, so this module starts no process; it reads
// the request file through the reader its caller gives.

import fs from "node:fs";
import path from "node:path";
import { requestProblems } from "../.github/scripts/lib/commits.mjs";
import { simpleCommands } from "./raw-build.mjs";

/// The only origin the client sends its token to.
const API = "https://api.github.com";
/// The paths of pull requests, as requestPath gives them: this repository's, and any repository's by
/// its id, since the rule does not know this one's; `pulls`, which a POST creates one at, and
/// `pulls/<number>`, which a PATCH edits.
const PULLS = /^\/(?:repos\/menteion\/mente|repositories\/\d+)\/pulls(\/\d+)?\/?$/;
/// The path of GitHub's GraphQL API.
const GRAPHQL = /^\/graphql\/?$/;
/// A word the shell or PowerShell may change before the client reads it: a variable or `"$@"`, a
/// command's output, PowerShell's splatting (`@a`), its escape (a backtick) or its stop-parsing token
/// (`--%`), and cmd's `%VAR%`.
const INDIRECT = /[$`@%]/;
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

/// Each call of the App's client in `text`: a simple command whose program is `node` and one of whose
/// words is `gh-app.mjs`, as its script or as the value of an option that loads it (`--import`, `-r`),
/// with the words after that word and the command as written, recursing into the shells it starts.
function clientCalls(text, depth = 0) {
  const found = [];
  for (const { text: line, words } of simpleCommands(String(text))) {
    const v = words.map((w) => w.value);
    const p = programAt(v);
    const at = v.findIndex((w, k) => k > p && programOf(w) === "gh-app.mjs");
    if (programOf(v[p] ?? "") === "node" && at > p) found.push({ found: line.slice(words[0].start, words[words.length - 1].end), rest: v.slice(at + 1) });
    if (depth >= DEPTH) continue;
    // A shell given a command line: bash's `-c` takes one word, PowerShell's `-Command` and cmd's
    // `/c` the rest of the line, as push.mjs reads them for git.
    for (let i = 0; i < v.length; i++) {
      const q = programOf(v[i]);
      const posix = ["bash", "sh", "zsh"].includes(q);
      const flag = posix ? /^-[a-z]*c$/ : ["powershell", "pwsh"].includes(q) ? /^-c(o(m(m(a(n(d)?)?)?)?)?)?$/i : q === "cmd" ? /^\/[a-z]*[ck]$/i : null;
      if (!flag) continue;
      const c = v.findIndex((w, k) => k > i && flag.test(w));
      if (c >= 0 && c + 1 < v.length) found.push(...clientCalls(posix || c === v.length - 2 ? v[c + 1] : line.slice(words[c + 1].start, words[words.length - 1].end), depth + 1));
      break;
    }
  }
  return found;
}

/// The path the client requests for `target`, as it builds the URL (a full URL as given, anything
/// else after its origin) and as the API reads it: its query and fragment dropped, dot segments
/// resolved, repeated slashes as one, decoded until it holds no more escapes, and lower-cased; or
/// null for a target the client sends nothing for (one Git Bash rewrote into a Windows path, or one
/// off its origin).
export function requestPath(target) {
  if (/^[A-Za-z]:[\\/]/.test(target)) return null;
  const url = target.startsWith("https://") ? target : `${API}${target.startsWith("/") ? "" : "/"}${target}`;
  if (!url.startsWith(`${API}/`)) return null;
  let at;
  try {
    at = new URL(url).pathname;
  } catch {
    return null;
  }
  for (let k = 0; k < 8; k++) {
    let decoded;
    try {
      decoded = decodeURIComponent(at);
    } catch {
      break;
    }
    const next = new URL(decoded.replace(/[\\/]+/g, "/").replace(/[?#]/g, encodeURIComponent), API).pathname;
    if (next === at) break;
    at = next;
  }
  return at.replace(/\/+/g, "/").toLowerCase();
}

/// The texts of pull requests a GraphQL request sends: none unless its query names
/// `createPullRequest` or `updatePullRequest`; otherwise whether it creates one, and every `title`
/// and `body` it sends, as an argument written inline (a string, a block string, null or a
/// variable) or as a field of its variables; or why the rule cannot read one. It reads them all, of
/// whatever mutation, since it does not know which pull request or repository an id names.
export function graphqlTexts(request) {
  if (request === null || typeof request !== "object" || typeof request.query !== "string") return { found: [] };
  const named = [...request.query.matchAll(/\b(create|update)PullRequest\b/gi)];
  if (named.length === 0) return { found: [] };
  const variables = request.variables !== null && typeof request.variables === "object" ? request.variables : {};
  const found = [];
  const walk = (value) => {
    if (Array.isArray(value)) value.forEach(walk);
    else if (value !== null && typeof value === "object") {
      for (const [key, x] of Object.entries(value)) {
        if ((key === "title" || key === "body") && (typeof x === "string" || x === null)) found.push([key, x]);
        else walk(x);
      }
    }
  };
  walk(variables);
  for (const m of request.query.matchAll(/(?<![$\w])(title|body)\s*:\s*/g)) {
    const after = request.query.slice(m.index + m[0].length);
    const unread = { why: `names a pull request's ${m[1]} in a form this rule cannot read` };
    if (after.startsWith('"""')) {
      const end = after.indexOf('"""', 3);
      if (end < 0) return unread;
      found.push([m[1], after.slice(3, end).replace(/\\"""/g, '"""')]);
    } else if (after.startsWith('"')) {
      const s = /^"((?:[^"\\\n]|\\.)*)"/.exec(after);
      if (!s) return unread;
      try {
        found.push([m[1], JSON.parse(`"${s[1]}"`)]);
      } catch {
        return unread;
      }
    } else if (after.startsWith("$")) {
      const name = /^\$(\w+)/.exec(after)?.[1];
      const value = name === undefined ? undefined : variables[name];
      if (typeof value === "string" || value === null) found.push([m[1], value]);
      else if (value !== undefined) return unread;
    } else if (/^null\b/.test(after)) {
      found.push([m[1], null]);
    } else {
      return unread;
    }
  }
  return { creates: named.some((m) => m[1].toLowerCase() === "create"), found };
}

/// The file a request file's name reads as on `platform`, or why the rule cannot read it: a name the
/// shell may change (a variable, a home directory, a command, a splat, cmd's `%VAR%`), a backslash,
/// which bash takes for an escape, or a path that is not absolute refuses. On Windows the name is a
/// drive's, `C:/...`: Git Bash rewrites `/c/...` into `C:/...`, but node, given it unchanged under
/// MSYS_NO_PATHCONV or from PowerShell, opens `\c\...` on the current drive, so the rule cannot know
/// which file the client sends.
function requestFile(name, platform) {
  if (INDIRECT.test(name) || name.startsWith("~")) return { why: "names its request file through the shell, which this rule cannot read: name it by its literal absolute path" };
  if (name.includes("\\")) return { why: "names its request file with a backslash, which bash reads as an escape: name it with forward slashes" };
  if (platform === "win32") return /^[a-zA-Z]:\//.test(name) ? { file: name } : { why: "names its request file by a path that is not a drive's: name it as C:/..., which every shell passes unchanged" };
  return name.startsWith("/") ? { file: name } : { why: "names its request file by a relative path: name it by its absolute path" };
}

/// Why the rule refuses `command`, a request that sends the text of a pull request that fails the
/// Commits check's rules, or one whose text it cannot read; or null. `readFile` reads a request file
/// as text.
export function bodyRefusal(command, { readFile = (f) => fs.readFileSync(f, "utf8"), platform = process.platform } = {}) {
  for (const { found, rest } of clientCalls(command)) {
    // A method written as one that sends no text of a pull request (GET, PUT, DELETE) is no concern.
    const api = rest.indexOf("api");
    const verb = api >= 0 ? String(rest[api + 1] ?? "") : "";
    if (api >= 0 && !INDIRECT.test(verb) && !["POST", "PATCH"].includes(verb.toUpperCase())) continue;
    // Any other word is read as written, but the output's name and the request file's, which
    // requestFile reads: a word the shell may change could be the method, the route, or
    // `--data-file` itself.
    const out = rest.indexOf("--out");
    const named = rest.indexOf("--data-file");
    if (rest.some((w, k) => !(out >= 0 && k === out + 1) && !(named >= 0 && k === named + 1) && INDIRECT.test(w))) return `"${found}" names a word of its call through the shell, which this rule cannot read: write the method, the route and --data-file as they are`;
    // The client sends a file only with `--data-file`; a call that sends none sends no text.
    if (api < 0 || named < 0) continue;
    // The client's own reading: its method, upper-cased, its route, and the word after the first
    // `--data-file` after them, which it sends as the request's body.
    const args = rest.slice(api + 1);
    const method = String(args[0] ?? "").toUpperCase();
    const route = requestPath(args[1] ?? "");
    if (route === null) continue;
    const pulls = PULLS.exec(route);
    const graphql = method === "POST" && GRAPHQL.test(route);
    const creates = pulls !== null && method === "POST" && pulls[1] === undefined;
    if (!graphql && !creates && !(pulls !== null && method === "PATCH" && pulls[1] !== undefined)) continue;
    const at = args.indexOf("--data-file");
    if (at < 0 || at + 1 >= args.length) continue;
    const name = args[at + 1];
    const { file, why } = requestFile(name, platform);
    if (why) return `"${found}" ${why}`;
    // Written and sent on one line, the file may change after this rule reads it.
    const base = path.posix.basename(name).toLowerCase();
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
    let problems;
    if (graphql) {
      const texts = graphqlTexts(request);
      if (texts.why) return `"${found}" ${texts.why}`;
      problems = texts.found.flatMap(([what, value]) => requestProblems({ [what]: value }, { creates: false }));
      if (texts.creates && !texts.found.some(([what]) => what === "body")) problems.push(...requestProblems({}, { creates: true }));
    } else {
      problems = requestProblems(request, { creates });
    }
    if (problems.length > 0) return `"${found}" sends a pull request's text that fails the Commits check's rules, which it would meet on GitHub:\n${[...new Set(problems)].map((p) => `- ${p}`).join("\n")}\nEdit ${name}, check it with node .github/scripts/commits-body.mjs ${name}, and send it again`;
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
