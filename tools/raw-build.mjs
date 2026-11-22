// What the hook a Claude Code session of this repository runs before each shell command decides
// (tools/raw-build-guard.mjs is the hook): whether the command would build, test or run the
// repository's Rust code outside the local limits, with cargo itself or a binary under target/, and
// what to run instead, as a diagnostic names its fix (tools/local-limits.mjs; the reset of
// 2026-11-22).
//
// It is a reading aid, not the wall: it reads the command's text, split into simple commands by a
// small tokenizer of bash and PowerShell, and refuses the common forms. The wall is cargo's runner
// on the laptop, tools/capped.mjs, which runs every test binary cargo starts under the limits
// however cargo was started, and which the tools refuse to run without (requireRunner in
// tools/local-limits.mjs; the review of 8f03074). Comments, heredoc bodies, here-strings and
// quoted text are no command to it, and a cargo command it does not know as harmless is refused.
// The workflow policy loads it and tests it, so this module starts no process and reads nothing.

import path from "node:path";
import { SYSTEM_POWERSHELL, storePowerShell } from "./store-apps.mjs";

/// The cargo commands that build nothing and run nothing of the repository's: every other one,
/// `test`, `llvm-cov`, `fix` or one not yet written, is refused.
export const CARGO_HARMLESS = ["fmt", "metadata", "tree", "version", "-V", "--version", "help", "-h", "--help", "--list", "locate-project", "pkgid", "search", "fetch", "generate-lockfile", "update", "add", "remove", "new", "init", "read-manifest", "verify-project", "config", "info", "report", "clean", "owner", "login", "logout", "yank", "vendor"];

// cargo's own options before its command: alone, and with a value.
const CARGO_FLAGS = new Set(["-q", "--quiet", "-v", "--verbose", "--locked", "--offline", "--frozen"]);
const CARGO_VALUED = new Set(["--color", "-Z", "--config", "-C"]);
// Words that start a simple command before the program: the shell's keywords.
const KEYWORDS = new Set(["then", "do", "else", "elif", "if", "while", "until", "!", "time"]);
// Programs that run the rest of their words as a command, after their own options.
const PREFIXES = new Set(["command", "exec", "nohup", "sudo", "xargs"]);
const DEPTH = 4;

/// The simple commands of `text`, a bash or PowerShell command line: each { text, words }, a word
/// { value, start, end } with its value as the shell reads it and its place in `text` as written.
/// A comment, a heredoc's body and a PowerShell here-string make no word; `;`, `&`, `|`, `(`, `)`,
/// `$(`, a newline, and `{` or `}` standing alone separate commands; a backslash escapes only a
/// character the shell would read otherwise, so that a Windows path keeps its own.
export function simpleCommands(text) {
  const commands = [];
  let words = [];
  let word = null;
  const heredocs = [];
  const n = text.length;
  let i = 0;
  const endWord = () => {
    if (word) {
      word.end = i;
      words.push(word);
      word = null;
    }
  };
  const endCommand = () => {
    endWord();
    if (words.length > 0) commands.push({ text, words });
    words = [];
  };
  const add = (s, at = i) => {
    if (!word) word = { value: "", start: at, end: at };
    word.value += s;
  };
  while (i < n) {
    const c = text[i];
    if (c === "\n") {
      endCommand();
      i++;
      // The bodies of the heredocs this line opened, up to each one's delimiter line.
      while (heredocs.length > 0) {
        const { delimiter, tabs } = heredocs.shift();
        while (i < n) {
          let e = text.indexOf("\n", i);
          if (e < 0) e = n;
          let line = text.slice(i, e).replace(/\r$/, "");
          if (tabs) line = line.replace(/^\t+/, "");
          i = e + 1;
          if (line === delimiter) break;
        }
      }
      continue;
    }
    if (c === " " || c === "\t" || c === "\r") {
      endWord();
      i++;
      continue;
    }
    if (c === "#" && !word) {
      while (i < n && text[i] !== "\n") i++;
      continue;
    }
    if (c === "'") {
      const e = text.indexOf("'", i + 1);
      const end = e < 0 ? n : e;
      add(text.slice(i + 1, end));
      i = end + 1;
      continue;
    }
    if (c === '"') {
      const at = i;
      let v = "";
      let j = i + 1;
      while (j < n && text[j] !== '"') {
        if ((text[j] === "\\" || text[j] === "`") && j + 1 < n && '"\\`$'.includes(text[j + 1])) {
          v += text[j + 1];
          j += 2;
          continue;
        }
        v += text[j];
        j++;
      }
      add(v, at);
      i = j + 1;
      continue;
    }
    if (c === "@" && !word && (text[i + 1] === "'" || text[i + 1] === '"') && /^\r?\n/.test(text.slice(i + 2, i + 4))) {
      const close = `\n${text[i + 1]}@`;
      const e = text.indexOf(close, i + 2);
      add("");
      i = e < 0 ? n : e + close.length;
      continue;
    }
    if (c === "\\" && i + 1 < n) {
      const d = text[i + 1];
      if (d === "\n") {
        i += 2;
        continue;
      }
      if (" \t'\"\\;&|<>()$`#{}".includes(d)) {
        add(d);
        i += 2;
        continue;
      }
      add(c);
      i++;
      continue;
    }
    if (c === "<" && text[i + 1] === "<" && text[i + 2] !== "<") {
      endWord();
      let j = i + 2;
      let tabs = false;
      if (text[j] === "-") {
        tabs = true;
        j++;
      }
      while (text[j] === " " || text[j] === "\t") j++;
      let delimiter = "";
      while (j < n && !" \t\r\n;&|<>()".includes(text[j])) {
        if (text[j] === "'" || text[j] === '"') {
          const e = text.indexOf(text[j], j + 1);
          delimiter += text.slice(j + 1, e < 0 ? n : e);
          j = e < 0 ? n : e + 1;
          continue;
        }
        if (text[j] !== "\\") delimiter += text[j];
        j++;
      }
      if (delimiter) heredocs.push({ delimiter, tabs });
      i = j;
      continue;
    }
    if (c === ">" || c === "<") {
      // A redirection, with the descriptor it may take: 2>&1, >&2, &> stays one word.
      const m = /^[<>]+&?[0-9-]*/.exec(text.slice(i));
      add(m[0]);
      i += m[0].length;
      continue;
    }
    if (c === "$" && text[i + 1] === "(") {
      endCommand();
      i += 2;
      continue;
    }
    if (";|&()".includes(c) || ((c === "{" || c === "}") && !word)) {
      endCommand();
      i++;
      continue;
    }
    add(c);
    i++;
  }
  endCommand();
  return commands;
}

/// A program's name as a word gives it: its file's name, without .exe, in lower case, as Windows
/// finds it.
const programOf = (value) => path.win32.basename(value).toLowerCase().replace(/\.exe$/, "");

/// What one simple command would run outside the limits, and what to run instead, or null.
function verdict({ text, words }, depth) {
  const v = (k) => words[k]?.value ?? "";
  const slice = (from) => (from < words.length ? text.slice(words[from].start, words[words.length - 1].end) : "");
  // The rest of the words as a command line of its own: one word's value, or the words as written.
  const inner = (from) => (from === words.length - 1 ? v(from) : slice(from));
  let i = 0;
  for (;;) {
    while (i < words.length && (KEYWORDS.has(v(i)) || /^[A-Za-z_][A-Za-z0-9_]*=/.test(v(i)))) i++;
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
    if (depth < DEPTH && p === "cmd") {
      const at = words.findIndex((w, k) => k > i && /^\/[a-z]*[ck]$/i.test(w.value));
      return at < 0 ? null : refusalIn(inner(at + 1), depth + 1);
    }
    if (depth < DEPTH && ["bash", "sh", "zsh"].includes(p)) {
      const at = words.findIndex((w, k) => k > i && /^-[a-z]*c$/.test(w.value));
      return at < 0 ? null : refusalIn(v(at + 1), depth + 1);
    }
    if (depth < DEPTH && ["powershell", "pwsh"].includes(p)) {
      const at = words.findIndex((w, k) => k > i && /^-c(o(m(m(a(n(d)?)?)?)?)?)?$/i.test(w.value));
      return at < 0 ? null : refusalIn(inner(at + 1), depth + 1);
    }
    break;
  }
  if (i >= words.length) return null;
  const program = programOf(v(i));
  // tools/capped.mjs asked to run the Store's pwsh, which would take every process it starts out of
  // the job: the same command with Windows PowerShell in its place (Norman, 2026-11-22). Its command
  // is the word after its first `--`, by either separator of a path; tools/cargo.mjs and mn.mjs run
  // cargo and mn, whatever their arguments name, so a test filter `--skip pwsh` is no program (the
  // review of 55c40e7).
  if (program === "node" && /(^|[\\/])tools[\\/]capped\.mjs$/i.test(v(i + 1))) {
    const dash = words.findIndex((w, k) => k > i + 1 && w.value === "--");
    const at = dash < 0 ? -1 : dash + 1;
    if (at < 0 || at >= words.length || !storePowerShell(v(at))) return null;
    const end = words[words.length - 1].end;
    return { found: slice(i), instead: `${text.slice(words[i].start, words[at].start)}${SYSTEM_POWERSHELL}${text.slice(words[at].end, end)}`, reason: "would start the Store's pwsh inside the job, which takes every process it starts out of it" };
  }
  if (program === "cargo") {
    let j = i + 1;
    while (j < words.length) {
      const a = v(j);
      if (a.startsWith("+") || CARGO_FLAGS.has(a) || /^-v+$/.test(a) || /^--(color|config)=/.test(a) || /^-Z./.test(a)) j++;
      else if (CARGO_VALUED.has(a)) j += 2;
      else break;
    }
    if (j >= words.length || CARGO_HARMLESS.includes(v(j))) return null;
    return { found: slice(i), instead: `node tools/cargo.mjs ${slice(i + 1)}` };
  }
  // A binary under target/, for the host or a target triple: target/debug/x, target/<triple>/release/x.
  const parts = v(i).split(/[\\/]/);
  const t = parts.findIndex((s) => s.toLowerCase() === "target");
  const profile = t < 0 ? -1 : parts.slice(t + 1, t + 3).findIndex((s) => /^(debug|release)$/i.test(s));
  if (profile >= 0 && parts.length > t + 2 + profile && parts.at(-1) !== "") {
    const rest = slice(i + 1);
    return { found: slice(i), instead: program === "mn" ? `node tools/mn.mjs${rest ? ` ${rest}` : ""}` : `node tools/cargo.mjs test${rest ? ` -- ${rest}` : ""}` };
  }
  return null;
}

function refusalIn(text, depth) {
  for (const command of simpleCommands(String(text))) {
    const why = verdict(command, depth);
    if (why) return why;
  }
  return null;
}

/// Why `command` is refused, and what to run instead: { found, instead }, or null when it runs no
/// raw build that this reading sees. `found` is the simple command as written; `instead` is the
/// same command through tools/cargo.mjs, or, for mn itself, tools/mn.mjs.
export function refusal(command) {
  return refusalIn(command, 0);
}

/// The refusal's words for the session: what would run outside the limits, or what else it would
/// do, and what to run.
export function message({ found, instead, reason = "would run outside the local limits" }) {
  return `"${found}" ${reason}. Run this instead: ${instead}\n(tools/cargo.mjs and tools/mn.mjs run each process in a job object that bounds its memory and time, and hold the machine to two builds at once: tools/local-limits.mjs.)`;
}

/// What the hook answers to `input`, its event as read from standard input: status 0 to let the
/// command run, or 2 to refuse it, with the words for the session. Any fault of its own refuses, so
/// that the hook fails closed: an event it cannot read, or one with no command.
export function decide(input) {
  let event;
  try {
    event = JSON.parse(input);
  } catch (e) {
    return { status: 2, words: `the raw-build guard could not read its event, so the command did not run: ${e.message}` };
  }
  const command = event?.tool_input?.command;
  if (typeof command !== "string") return { status: 2, words: "the raw-build guard found no command in its event, so the command did not run" };
  const why = refusal(command);
  return why ? { status: 2, words: message(why) } : { status: 0, words: "" };
}
