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
import { mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { odinProgram, parseSignature, rustProgram, type Lang, type Signature } from "./languages.ts";
import { outcome, type Case, type CaseResult, type Task } from "./tasks.ts";
import { own, setOwn } from "../own.mjs";
import { quotient } from "../quotient.mjs";

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
  /**
   * The output directory, in which every build's directory lies. With the sandbox it lies under the
   * sandbox's runs root, which the sandbox covers all but the build's own directory, so no build
   * can read another run's folder, a held-out run's prompts, programs and records among them.
   */
  out: string;
}

/** The sandbox's configuration, which benchmark/harness/lock-sudo.sh writes as root. */
export const SANDBOX_CONF = "/etc/mente-sandbox.conf";

/**
 * What the sandbox covers, read from its configuration: the directories it hides (the workspace,
 * where the held-out set is checked out, and the broker's directory) and the runs root, under which
 * every working directory it takes must lie. The sandbox's root half takes no path but the working
 * directory, so this is what the harness lays its output out by.
 */
export function sandboxRoots(text: string): { hide: string[]; runs: string } {
  const hide: string[] = [];
  let runs = "";
  for (const line of text.split("\n")) {
    const words = line.trim().split(/\s+/);
    if (words.length === 2 && own(words, 0) === "hide") hide.push(own(words, 1));
    else if (words.length === 2 && own(words, 0) === "runs") runs = own(words, 1);
  }
  if (runs === "") throw new Error(`${SANDBOX_CONF} names no runs root: benchmark/harness/lock-sudo.sh writes it`);
  return { hide, runs };
}

/** The sandbox's roots on this runner, or an error saying the sandbox is not set up. */
export function readSandboxRoots(): { hide: string[]; runs: string } {
  let text = "";
  try {
    text = readFileSync(SANDBOX_CONF, "utf8");
  } catch {
    throw new Error(`no ${SANDBOX_CONF}: the sandbox is not set up; benchmark/harness/lock-sudo.sh sets it up`);
  }
  return sandboxRoots(text);
}

/** This repository's root, which the sandbox hides from agent code. */
export const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

/**
 * A command, wrapped so it runs through the sandbox when one is set: `sandbox.sh <workdir> <cmd>
 * <args>`, run with bash. The process starts in its build directory, so the script,
 * the paths, and a command given as a path (such as `target/release/mn`) are made absolute against
 * the driver's own directory; a bare command name is left for PATH to find.
 */
export function sandboxed(sandbox: string, workdir: string, cmd: string, args: string[]): [string, string[]] {
  const command = /[\\/]/.test(cmd) ? resolve(cmd) : cmd;
  if (!sandbox) return [command, args];
  return [BASH, [resolve(sandbox), resolve(workdir), command, ...args]];
}

/** A path as the file system resolves it, links and all, or as written when it is not there yet. */
function real(path: string): string {
  try {
    return realpathSync(path);
  } catch {
    return resolve(path);
  }
}

/**
 * Whether `path` is one of `roots` or lies inside one, each as the file system resolves it: a link
 * inside a root that points out of it lies outside (the review of aafe59b: a held-out-set link to
 * $RUNNER_TEMP passed for the hidden workspace).
 */
export function insideAny(path: string, roots: string[]): boolean {
  const p = real(path);
  return roots.some((r) => {
    const root = real(r);
    return p === root || p.startsWith(root.endsWith(sep) ? root : root + sep);
  });
}

/**
 * Refuses an output directory the sandbox would not take: every build's working directory lies in
 * it, and the sandbox takes a working directory only strictly under its runs root.
 */
export function checkUnderRuns(out: string, roots: { runs: string }): void {
  const p = resolve(out);
  if (p === resolve(roots.runs) || !insideAny(p, [roots.runs])) throw new Error(`with --sandbox, --out must lie under the sandbox's runs root ${roots.runs}; ${p} does not`);
}

/**
 * The toolchain's own rustc, from its sysroot, so the rustup proxy, which may want to write to
 * RUSTUP_HOME, never runs under the sandbox's read-only root. Falls back to `rustc` as given.
 */
export async function rustcInSysroot(rustc: string, toolchain: string): Promise<string> {
  const env = childEnv(toolchain ? { RUSTUP_TOOLCHAIN: toolchain } : {});
  const p = await runProcess(rustc, ["--print", "sysroot"], { env, timeoutMs: 60_000 });
  const sysroot = p.stdout.trim();
  if (p.status !== 0 || sysroot === "") return rustc;
  return join(sysroot, "bin", process.platform === "win32" ? "rustc.exe" : "rustc");
}

export interface Proc {
  status: number | null;
  signal: string | null;
  stdout: string;
  stderr: string;
  timedOut: boolean;
}

const OUTPUT_CAP = 1_000_000;

/** A variable's name that could hold a secret: no process the harness starts is given one. */
const SECRET_NAME = /key|token|secret|password|credential/i;

/**
 * The environment of every process the harness starts, the agent's programs among them: this
 * one's without any variable whose name looks like a secret, such as ANTHROPIC_API_KEY.
 */
export function childEnv(extra: Record<string, string> = {}): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {};
  for (const [name, value] of Object.entries(process.env)) {
    if (!SECRET_NAME.test(name)) setOwn(env, name, value);
  }
  return { ...env, ...extra };
}

/** The benchmark's sandbox, the repository's own: the one confinement of what runs beside a secret. */
export const SANDBOX = join(REPOSITORY, "benchmark", "harness", "sandbox.sh");
/**
 * The shell that runs it, by its path, never by a search of PATH, where a directory of the
 * runner's user (~/.local/bin) comes before /usr/bin, as test-sandbox.sh names its programs.
 */
export const BASH = "/usr/bin/bash";

/** Whether two paths name the same file, the case of a Windows path aside. */
const samePath = (a: string, b: string) => (process.platform === "win32" ? resolve(a).toLowerCase() === resolve(b).toLowerCase() : resolve(a) === resolve(b));

/**
 * Whether a command is one the harness may start while a secret is in reach: the benchmark's own
 * sandbox, whatever it runs inside, as `sandboxed(...)` gives it; or a toolchain's probe of its
 * version or sysroot, with exactly those arguments, which runs outside the sandbox because the
 * rustup proxy may write to RUSTUP_HOME. Nothing else: not mn, not a shell, not another script
 * than SANDBOX (the review of b2be87b: refusing mn by its name let `bash -c "$mn ..."` and a copy
 * of mn under another name through).
 */
export function namedCommand(cmd: string, args: string[]): boolean {
  if (cmd === BASH) return args.length >= 3 && samePath(own(args, 0), SANDBOX);
  const tool = own(/(?:^|[\\/])(rustc|odin)(?:\.exe)?$/.exec(cmd), 1);
  const probe = args.join(" ");
  return (tool === "rustc" && (probe === "--version" || probe === "--print sysroot")) || (tool === "odin" && probe === "version");
}

/** Whether a secret is in this process's reach: a variable whose name looks like one, set. */
const secretInReach = () => Object.entries(process.env).some(([name, value]) => SECRET_NAME.test(name) && value);
/**
 * Whether one was when this module loaded: a secret deleted from the environment later is still in
 * this process's memory, so the rule holds from then on (the review of 41a54b6).
 */
const secretAtLoad = secretInReach();

/**
 * Runs a command to its end, or kills it at the time limit: the one place the harness of a job
 * that holds the key starts a process (the policy holds the other modules to it). Whatever
 * environment it is given, no variable that could hold a secret reaches the child. And while this
 * process holds such a variable, it starts only a named command, `namedCommand`: so `mn`, code of
 * the branch, and the agents' programs run only through the sandbox (the final review of PR 137
 * found mn run directly, and the review of b2be87b mn run by a shell).
 */
export function runProcess(
  cmd: string,
  args: string[],
  opts: { cwd?: string; env?: NodeJS.ProcessEnv; timeoutMs: number },
): Promise<Proc> {
  if ((secretAtLoad || secretInReach()) && !namedCommand(cmd, args)) {
    return Promise.reject(new Error(`${cmd} is no command the harness starts beside a secret: only the benchmark's sandbox and the toolchains' probes are`));
  }
  const env = Object.fromEntries(Object.entries(opts.env ?? childEnv()).filter(([name]) => !SECRET_NAME.test(name)));
  return new Promise((done) => {
    let stdout = "";
    let stderr = "";
    let timedOut = false;
    const child = spawn(cmd, args, { cwd: opts.cwd, env, stdio: ["ignore", "pipe", "pipe"] });
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
  while (lines.length > 0 && own(lines, lines.length - 1).trim() === "") lines.pop();
  return lines;
}

/**
 * What a run printed, in the case's own form: the result is the last line before the `leaves`
 * lines, so a line the agent's code printed itself is not taken for it.
 */
export function printedOutcome(c: Case, stdout: string, mutCount: number): string {
  const lines = stdout.replace(/\r\n/g, "\n").split("\n");
  while (lines.length > 0 && own(lines, lines.length - 1) === "") lines.pop();
  const tail = lines.slice(Math.max(lines.length - 1 - mutCount, 0));
  const result = own(tail, 0) ?? "";
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
    if (panic) return `panicked at ${own(panic, 1)}: ${own(panic, 2)}`;
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
  if (p.timedOut) return { ...c, got: `ran longer than ${quotient(RUN_TIMEOUT_MS, 1000)} s`, pass: false, fault: "timeout" };
  if (p.status !== 0) return { ...c, got: trapText(lang, p, dir), pass: false, fault: "trap" };
  const got = printedOutcome(c, p.stdout, mutCount);
  return got === c.expected ? { ...c, got, pass: true } : { ...c, got, pass: false, fault: "wrong result" };
}

/** A Mente case's result from `mn run`. */
export function menteCaseResult(c: Case, p: Proc): CaseResult {
  if (p.timedOut) return { ...c, got: `ran longer than ${quotient(RUN_TIMEOUT_MS, 1000)} s`, pass: false, fault: "timeout" };
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

/**
 * The failure class `mn explain` gives each code, asked once per code: through the sandbox, as
 * every other run of `mn`, which is code of the branch and runs beside the process that holds the
 * key (the final review of PR 137 found this one run directly), in an empty directory of its own
 * in the output directory. With no sandbox it refuses to run.
 */
const classes = new Map<string, Promise<string>>();
export function failureClass(tools: Tools, code: string): Promise<string> {
  if (!tools.sandbox) return Promise.reject(new Error("mn explain would run outside the sandbox"));
  if (!classes.has(code)) {
    const dir = resolve(tools.out || ".", "explain");
    mkdirSync(dir, { recursive: true });
    const [c, a] = sandboxed(tools.sandbox, dir, tools.mn, ["explain", code]);
    classes.set(
      code,
      runProcess(c, a, { cwd: dir, env: childEnv(), timeoutMs: 30_000 }).then((p) => own(/Failure class: ([^.]+)\./.exec(p.stdout), 1) ?? "unknown"),
    );
  }
  return classes.get(code) ?? Promise.resolve("unknown");
}

/** A task's signature, read once. */
export function signatureOf(task: Task): Signature {
  return parseSignature(task.signature);
}
