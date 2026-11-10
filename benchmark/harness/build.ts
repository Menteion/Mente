// Building an agent's program and running its cases in Mente, Rust or Odin, for the benchmark
// harness (benchmark/README.md, "Method").
//
// Mente: `mn check`, then each case on the reference interpreter with `mn run --call`, or as
// native code with `--native`. Rust: `rustc` from the toolchain `toolchains.json` pins, with
// overflow checks and debug assertions on, as in a debug build, at `-C opt-level=1` so a hidden
// test's loop runs in its time limit. Odin: `odin build` with its defaults, which keep bounds
// checks on; Odin's integers wrap on overflow, which is the language's own definition, so the
// harness adds no check. A program that panics or traps fails its case, and so does one that runs
// longer than the time limit.

import { spawn } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { odinProgram, parseSignature, rustProgram, type Lang, type Signature } from "./languages.ts";
import { outcome, type Case, type CaseResult, type Task } from "./tasks.ts";

/** The time one case may run, in milliseconds. */
export const RUN_TIMEOUT_MS = 10_000;
/** The time a build may take, in milliseconds. */
export const BUILD_TIMEOUT_MS = 180_000;

export const RUST_FLAGS = ["--edition", "2024", "-C", "opt-level=1", "-C", "overflow-checks=on", "-C", "debug-assertions=on"];

/** The build commands, as results.json records them. */
export const BUILD_MODES: Record<Lang, string> = {
  mente: "mn check --json, then mn run <file> --call <case> on the reference interpreter (--native: as native code)",
  rust: `rustc ${RUST_FLAGS.join(" ")} main.rs, which includes the agent's solution.rs`,
  odin: "odin build <dir> -out:<exe>, with the defaults, which keep bounds checks",
};

export interface Tools {
  mn: string;
  /** Run Mente's cases as native code instead of on the reference interpreter. */
  native: boolean;
  rustc: string;
  /** The rustup toolchain, set as RUSTUP_TOOLCHAIN; empty to take rustc as it is. */
  rustToolchain: string;
  odin: string;
  /**
   * The sandbox wrapper (`sandbox.sh`) every agent build and agent program runs through, or empty
   * to run them directly. Set in CI only; a developer's machine runs them directly.
   */
  sandbox: string;
}

/**
 * A command, wrapped so it runs through the sandbox when one is set: `sandbox.sh <workdir> <cmd>
 * <args>`, run with bash. The process starts in its build directory, so the script, the working
 * directory and a command given as a path (such as `target/release/mn`) are made absolute against
 * the driver's own directory; a bare command name is left for PATH to find.
 */
export function sandboxed(sandbox: string, workdir: string, cmd: string, args: string[]): [string, string[]] {
  const command = /[\\/]/.test(cmd) ? resolve(cmd) : cmd;
  return sandbox ? ["bash", [resolve(sandbox), resolve(workdir), command, ...args]] : [command, args];
}

export interface Proc {
  status: number | null;
  signal: string | null;
  stdout: string;
  stderr: string;
  timedOut: boolean;
}

const OUTPUT_CAP = 1_000_000;

/**
 * The environment of every process the harness starts, the agent's programs among them: this
 * one's without any variable whose name looks like a secret, such as ANTHROPIC_API_KEY.
 */
export function childEnv(extra: Record<string, string> = {}): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {};
  for (const [name, value] of Object.entries(process.env)) {
    if (!/key|token|secret|password|credential/i.test(name)) env[name] = value;
  }
  return { ...env, ...extra };
}

/** Runs a command to its end, or kills it at the time limit. */
export function runProcess(
  cmd: string,
  args: string[],
  opts: { cwd?: string; env?: NodeJS.ProcessEnv; timeoutMs: number },
): Promise<Proc> {
  return new Promise((done) => {
    let stdout = "";
    let stderr = "";
    let timedOut = false;
    const child = spawn(cmd, args, { cwd: opts.cwd, env: opts.env ?? childEnv(), stdio: ["ignore", "pipe", "pipe"] });
    child.stdout.on("data", (d) => {
      if (stdout.length < OUTPUT_CAP) stdout += d;
    });
    child.stderr.on("data", (d) => {
      if (stderr.length < OUTPUT_CAP) stderr += d;
    });
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill("SIGKILL");
    }, opts.timeoutMs);
    child.on("error", (e) => {
      clearTimeout(timer);
      done({ status: null, signal: null, stdout, stderr: `${stderr}${String(e)}`, timedOut });
    });
    child.on("close", (status, signal) => {
      clearTimeout(timer);
      done({ status, signal, stdout, stderr, timedOut });
    });
  });
}

/** A compiler's output with the build directory taken out of its paths, as lines. */
export function cleanOutput(text: string, dir: string): string[] {
  const abs = resolve(dir);
  let out = text;
  for (const d of new Set([abs, abs.replace(/\\/g, "/"), dir, dir.replace(/\\/g, "/")])) {
    out = out.split(`${d}/`).join("").split(`${d}\\`).join("");
  }
  const lines = out.replace(/\r\n/g, "\n").split("\n");
  while (lines.length > 0 && lines[lines.length - 1].trim() === "") lines.pop();
  return lines;
}

/**
 * What a run printed, in the case's own form: the result is the last line before the `leaves`
 * lines, so a line the agent's code printed itself is not taken for it.
 */
export function printedOutcome(c: Case, stdout: string, mutCount: number): string {
  const lines = stdout.replace(/\r\n/g, "\n").split("\n");
  while (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  const tail = lines.slice(Math.max(lines.length - 1 - mutCount, 0));
  const result = tail[0] ?? "";
  const left = tail
    .slice(1)
    .map((l) => l.replace(/^leaves /, ""))
    .join(", ");
  if (c.value && c.leaves) return `${result} leaves ${left}`;
  return c.leaves ? left : result;
}

/** How a Rust or Odin program that did not finish normally stopped. */
export function trapText(lang: "rust" | "odin", p: Proc, dir = "."): string {
  const err = cleanOutput(p.stderr, dir).join("\n");
  if (lang === "rust") {
    const panic = /panicked at ([^\n]+?):\n([^\n]+)/.exec(err);
    if (panic) return `panicked at ${panic[1]}: ${panic[2]}`;
    if (/has overflowed its stack/.test(err)) return "overflowed its stack";
  }
  const last = err
    .split("\n")
    .filter((l) => l.trim() !== "")
    .pop();
  const how = p.signal ? `signal ${p.signal}` : `exit status ${p.status}`;
  return last ? `trapped (${how}): ${last.trim()}` : `stopped by ${how}`;
}

/** A case's result from the process that ran it. */
export function caseResult(lang: "rust" | "odin", c: Case, p: Proc, mutCount: number, dir = "."): CaseResult {
  if (p.timedOut) return { ...c, got: `ran longer than ${RUN_TIMEOUT_MS / 1000} s`, pass: false, fault: "timeout" };
  if (p.status !== 0) return { ...c, got: trapText(lang, p, dir), pass: false, fault: "trap" };
  const got = printedOutcome(c, p.stdout, mutCount);
  return got === c.expected ? { ...c, got, pass: true } : { ...c, got, pass: false, fault: "wrong result" };
}

/** A Mente case's result from `mn run`. */
export function menteCaseResult(c: Case, p: Proc): CaseResult {
  if (p.timedOut) return { ...c, got: `ran longer than ${RUN_TIMEOUT_MS / 1000} s`, pass: false, fault: "timeout" };
  const got = outcome(c, p.status, p.stdout, p.stderr);
  if (p.status !== 0) {
    return { ...c, got, pass: false, fault: /more steps than its limit/.test(got) ? "timeout" : "trap" };
  }
  return got === c.expected ? { ...c, got, pass: true } : { ...c, got, pass: false, fault: "wrong result" };
}

/** A built program: whether it built, what the compiler reported, and how to run a case. */
export interface Build {
  lang: Lang;
  ok: boolean;
  /** The compiler's report when the program does not build, as the agent is shown it. */
  diagnostics: string[];
  /** The codes of a Mente program's diagnostics, which give its failure classes. */
  codes: string[];
  /** The source file the agent's program was written to. */
  file: string;
  dir: string;
  exe: string;
}

interface MenteDiagnostic {
  code: string;
  message: string;
  primary: { line: number; column: number };
  fixes: { message: string }[];
}

/** What a run's build of the agent's program reports; the cases go into the program around it. */
export async function buildProgram(
  lang: Lang,
  where: string,
  program: string,
  sig: Signature,
  cases: Case[],
  tools: Tools,
): Promise<Build> {
  // Absolute, since each process starts in this directory: the source and executable paths the
  // commands are given must not depend on where they start.
  const dir = resolve(where);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  const exe = join(dir, process.platform === "win32" ? "main.exe" : "main");
  if (lang === "mente") {
    const file = join(dir, "solution.mn");
    writeFileSync(file, `module bench\n\n${program}`);
    const [c, a] = sandboxed(tools.sandbox, dir, tools.mn, ["check", "--json", file]);
    const p = await runProcess(c, a, { cwd: dir, env: childEnv(), timeoutMs: BUILD_TIMEOUT_MS });
    let diagnostics: MenteDiagnostic[];
    try {
      diagnostics = JSON.parse(p.stdout).diagnostics;
    } catch {
      throw new Error(`mn check gave no report for ${file}: ${p.stderr.trim()}`);
    }
    return {
      lang,
      ok: diagnostics.length === 0,
      diagnostics: diagnostics.map(
        (d) => `${d.primary.line}:${d.primary.column} ${d.code} ${d.message}` + d.fixes.map((f) => ` (fix: ${f.message})`).join(""),
      ),
      codes: diagnostics.map((d) => d.code),
      file,
      dir,
      exe: "",
    };
  }
  if (lang === "rust") {
    const file = join(dir, "solution.rs");
    writeFileSync(file, program);
    writeFileSync(join(dir, "main.rs"), rustProgram(sig, cases));
    const env = childEnv(tools.rustToolchain ? { RUSTUP_TOOLCHAIN: tools.rustToolchain } : {});
    const [c, a] = sandboxed(tools.sandbox, dir, tools.rustc, [...RUST_FLAGS, "-o", exe, "main.rs"]);
    const p = await runProcess(c, a, { cwd: dir, env, timeoutMs: BUILD_TIMEOUT_MS });
    const ok = p.status === 0 && !p.timedOut;
    return { lang, ok, diagnostics: ok ? [] : cleanOutput(p.stderr || p.stdout, dir), codes: [], file, dir, exe };
  }
  const file = join(dir, "solution.odin");
  writeFileSync(file, `package main\n\n${program}`);
  writeFileSync(join(dir, "harness.odin"), odinProgram(sig, cases));
  const exeName = process.platform === "win32" ? "main.exe" : "main";
  const [c, a] = sandboxed(tools.sandbox, dir, tools.odin, ["build", ".", `-out:${exeName}`]);
  const p = await runProcess(c, a, { cwd: dir, env: childEnv(), timeoutMs: BUILD_TIMEOUT_MS });
  const ok = p.status === 0 && !p.timedOut;
  return { lang, ok, diagnostics: ok ? [] : cleanOutput(`${p.stderr}${p.stdout}`, dir), codes: [], file, dir, exe };
}

/** Runs each case of a program that built; the cases are the ones its build was given. */
export async function runCases(build: Build, sig: Signature, cases: Case[], tools: Tools): Promise<CaseResult[]> {
  const results: CaseResult[] = [];
  const mutCount = sig.params.filter((p) => p.mut).length;
  for (const [i, c] of cases.entries()) {
    if (build.lang === "mente") {
      const args = ["run", build.file, "--call", c.call, ...(tools.native ? ["--native"] : [])];
      const [cmd, a] = sandboxed(tools.sandbox, build.dir, tools.mn, args);
      results.push(menteCaseResult(c, await runProcess(cmd, a, { cwd: build.dir, env: childEnv(), timeoutMs: RUN_TIMEOUT_MS * 6 })));
    } else {
      const [cmd, a] = sandboxed(tools.sandbox, build.dir, build.exe, [String(i)]);
      const p = await runProcess(cmd, a, { cwd: build.dir, env: childEnv(), timeoutMs: RUN_TIMEOUT_MS });
      results.push(caseResult(build.lang, c, p, mutCount, build.dir));
    }
  }
  return results;
}

/** How an adversarial task's run ended (benchmark/README.md, "Method"). */
export type Verdict = "caught at build" | "caught at run time" | "missed";

/**
 * The verdict on the last program of an adversarial task's run: caught at build when it does not
 * build, missed when a probe call returns a value, and caught at run time when every probe call
 * stops without one.
 */
export function verdictOf(built: boolean, probe: CaseResult[]): Verdict {
  if (!built) return "caught at build";
  return probe.some((r) => r.fault === "wrong result" || r.pass) ? "missed" : "caught at run time";
}

/** Removes a build's executable, which a record has no use for. */
export function dropExecutable(build: Build): void {
  if (build.exe) {
    for (const f of [build.exe, build.exe.replace(/\.exe$/, ".pdb")]) rmSync(f, { force: true });
  }
}

/** The failure class `mn explain` gives each code, asked once per code. */
const classes = new Map<string, Promise<string>>();
export function failureClass(mn: string, code: string): Promise<string> {
  if (!classes.has(code)) {
    classes.set(
      code,
      runProcess(mn, ["explain", code], { timeoutMs: 30_000 }).then((p) => /Failure class: ([^.]+)\./.exec(p.stdout)?.[1] ?? "unknown"),
    );
  }
  return classes.get(code) ?? Promise.resolve("unknown");
}

/** A task's signature, read once. */
export function signatureOf(task: Task): Signature {
  return parseSignature(task.signature);
}
