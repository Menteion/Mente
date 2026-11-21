// The hook a Claude Code session of this repository runs before each shell command: it refuses a
// command that builds, tests or runs the repository's Rust code with cargo itself, or runs a binary
// under target/, and names the command to run instead, as a diagnostic names its fix. The capped
// tools run each process in a job object that bounds its memory and time, and hold the machine to
// two builds at once (tools/local-limits.mjs; the reset of 2026-11-22).
//
// tools/raw-build-guard.mjs is the hook itself; this module decides.
//
// It reads the command's text, so it is a strong default, not a wall: text in quotes is no command
// to it. The workflow policy loads refusal() and tests it, so this module starts no process and
// reads nothing but its input.

import path from "node:path";

/// The cargo commands that build or run the repository's code: those the capped tools run instead.
export const CARGO_BUILDS = ["build", "b", "check", "c", "clippy", "test", "t", "run", "r", "bench", "nextest", "mutants", "fuzz", "doc", "d", "install", "miri", "rustc", "rustdoc"];

// Where a simple command starts: the line's start or a separator, then any variables it sets.
const START = String.raw`(?:^|[\n;&|(]|\bthen\b|\bdo\b|\belse\b)\s*(?:[A-Za-z_][A-Za-z0-9_]*=\S*\s+)*(?:&\s*)?`;
const CARGO = new RegExp(`${START}((?:[^\\s;&|]*[\\\\/])?cargo(?:\\.exe)?)\\s+(?:\\+\\S+\\s+)?(?:${CARGO_BUILDS.join("|")})(?=\\s|$|[;&|)])`, "d");
const BINARY = new RegExp(`${START}([^\\s;&|]*target[\\\\/](?:[^\\s;&|\\\\/]+[\\\\/])?(?:debug|release)[\\\\/][^\\s;&|]+)`, "d");

/// Why `command` is refused, and what to run instead: { found, instead }, or null when it runs no
/// raw build. `found` is the simple command that would run outside the limits, as written; `instead`
/// is the same command through tools/cargo.mjs, or, for mn itself, tools/mn.mjs.
export function refusal(command) {
  const text = String(command);
  // Quoted text is no command: blanked to the same length, so that positions stay the text's.
  const bare = text.replace(/'[^']*'|"(?:[^"\\]|\\.)*"/g, (q) => " ".repeat(q.length));
  const end = (from) => {
    const m = /[\n;&|)]/.exec(bare.slice(from));
    return m ? from + m.index : text.length;
  };
  const cargo = CARGO.exec(bare);
  if (cargo) {
    const [at, after] = cargo.indices[1];
    const stop = end(after);
    return { found: text.slice(at, stop).trim(), instead: `node tools/cargo.mjs${text.slice(after, stop).trimEnd()}` };
  }
  const binary = BINARY.exec(bare);
  if (binary) {
    const [at, after] = binary.indices[1];
    const stop = end(after);
    const rest = text.slice(after, stop).trimEnd();
    const name = path.win32.basename(path.posix.basename(text.slice(at, after))).replace(/\.exe$/i, "");
    return { found: text.slice(at, stop).trim(), instead: name === "mn" ? `node tools/mn.mjs${rest}` : `node tools/cargo.mjs test${rest ? ` --${rest}` : ""}` };
  }
  return null;
}

/// The refusal's words for the session: what would run outside the limits, and what to run.
export function message({ found, instead }) {
  return `"${found}" would run outside the local limits. Run this instead: ${instead}\n(tools/cargo.mjs and tools/mn.mjs run each process in a job object that bounds its memory and time, and hold the machine to two builds at once: tools/local-limits.mjs.)`;
}

